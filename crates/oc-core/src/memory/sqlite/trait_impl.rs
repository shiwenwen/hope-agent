use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use std::sync::Arc;

use super::backend::{row_to_entry, scope_where, SqliteMemoryBackend};
use super::prompt::format_prompt_summary;
use crate::memory::helpers::{
    load_dedup_config, load_hybrid_search_config, load_temporal_decay_config,
};
use crate::memory::traits::{EmbeddingProvider, MemoryBackend};
use crate::memory::types::*;

// ── MemoryBackend Implementation ────────────────────────────────

impl MemoryBackend for SqliteMemoryBackend {
    fn add(&self, entry: NewMemory) -> Result<i64> {
        let conn = self.write_conn()?;
        let now = chrono::Utc::now().to_rfc3339();
        let tags_json = serde_json::to_string(&entry.tags)?;

        let (scope_type, scope_agent_id) = match &entry.scope {
            MemoryScope::Global => ("global", None),
            MemoryScope::Agent { id } => ("agent", Some(id.as_str())),
        };

        // Generate embedding: multimodal if attachment present + supported, else text-only
        let embedding = if let (Some(ref att_path), Some(ref att_mime)) =
            (&entry.attachment_path, &entry.attachment_mime)
        {
            self.generate_multimodal_embedding(&entry.content, att_path, att_mime)
        } else {
            self.generate_embedding(&entry.content)
        };
        let embedding_bytes: Option<Vec<u8>> = embedding
            .as_ref()
            .map(|v| v.iter().flat_map(|f| f.to_le_bytes()).collect());

        conn.execute(
            "INSERT INTO memories (memory_type, scope_type, scope_agent_id, content, tags, source, source_session_id, embedding, pinned, created_at, updated_at, attachment_path, attachment_mime)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                entry.memory_type.as_str(),
                scope_type,
                scope_agent_id,
                entry.content,
                tags_json,
                entry.source,
                entry.source_session_id,
                embedding_bytes,
                entry.pinned as i64,
                now,
                now,
                entry.attachment_path,
                entry.attachment_mime,
            ],
        )?;

        let row_id = conn.last_insert_rowid();

        // Insert into vec0 table if embedding was generated
        if let Some(ref emb_bytes) = embedding_bytes {
            let dims = self
                .embedding_dims
                .load(std::sync::atomic::Ordering::Relaxed);
            if dims > 0 {
                let _ = self.ensure_vec_table(&conn, dims);
                let _ = conn.execute(
                    "INSERT INTO memories_vec(rowid, embedding) VALUES (?1, ?2)",
                    params![row_id, emb_bytes],
                );
            }
        }

        Ok(row_id)
    }

    fn update(&self, id: i64, content: &str, tags: &[String]) -> Result<()> {
        let conn = self.write_conn()?;
        let now = chrono::Utc::now().to_rfc3339();
        let tags_json = serde_json::to_string(tags)?;

        // Regenerate embedding if provider is configured
        let embedding = self.generate_embedding(content);
        let embedding_bytes: Option<Vec<u8>> = embedding
            .as_ref()
            .map(|v| v.iter().flat_map(|f| f.to_le_bytes()).collect());

        let affected = conn.execute(
            "UPDATE memories SET content = ?1, tags = ?2, embedding = ?3, updated_at = ?4 WHERE id = ?5",
            params![content, tags_json, embedding_bytes, now, id],
        )?;

        if affected == 0 {
            anyhow::bail!("Memory with id {} not found", id);
        }

        // Update vec0 table
        if let Some(ref emb_bytes) = embedding_bytes {
            let dims = self
                .embedding_dims
                .load(std::sync::atomic::Ordering::Relaxed);
            if dims > 0 {
                let _ = self.ensure_vec_table(&conn, dims);
                // Delete old vector + insert new
                let _ = conn.execute("DELETE FROM memories_vec WHERE rowid = ?1", params![id]);
                let _ = conn.execute(
                    "INSERT INTO memories_vec(rowid, embedding) VALUES (?1, ?2)",
                    params![id, emb_bytes],
                );
            }
        }

        Ok(())
    }

    fn toggle_pin(&self, id: i64, pinned: bool) -> Result<()> {
        let conn = self.write_conn()?;
        let now = chrono::Utc::now().to_rfc3339();
        let affected = conn.execute(
            "UPDATE memories SET pinned = ?1, updated_at = ?2 WHERE id = ?3",
            params![pinned as i64, now, id],
        )?;
        if affected == 0 {
            anyhow::bail!("Memory with id {} not found", id);
        }
        Ok(())
    }

    fn delete(&self, id: i64) -> Result<()> {
        let conn = self.write_conn()?;
        // Delete from vec0 first (if table exists)
        let _ = conn.execute("DELETE FROM memories_vec WHERE rowid = ?1", params![id]);
        conn.execute("DELETE FROM memories WHERE id = ?1", params![id])?;
        Ok(())
    }

    fn get(&self, id: i64) -> Result<Option<MemoryEntry>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, memory_type, scope_type, scope_agent_id, content, tags, source, source_session_id, pinned, created_at, updated_at
             FROM memories WHERE id = ?1",
        )?;

        let entry = stmt.query_row(params![id], row_to_entry).optional()?;
        Ok(entry)
    }

    fn list(
        &self,
        scope: Option<&MemoryScope>,
        types: Option<&[MemoryType]>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<MemoryEntry>> {
        let conn = self.read_conn()?;

        let (scope_clause, mut scope_params) = scope_where(scope, None);

        let type_clause = if let Some(types) = types {
            if types.is_empty() {
                "1=1".to_string()
            } else {
                let placeholders: Vec<String> = types.iter().map(|_| "?".to_string()).collect();
                format!("memory_type IN ({})", placeholders.join(", "))
            }
        } else {
            "1=1".to_string()
        };

        let sql = format!(
            "SELECT id, memory_type, scope_type, scope_agent_id, content, tags, source, source_session_id, pinned, created_at, updated_at
             FROM memories
             WHERE {} AND {}
             ORDER BY pinned DESC, updated_at DESC
             LIMIT ? OFFSET ?",
            scope_clause, type_clause
        );

        let mut stmt = conn.prepare(&sql)?;

        // Build params: scope_params + type_params + limit + offset
        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        all_params.append(&mut scope_params);
        if let Some(types) = types {
            for t in types {
                all_params.push(Box::new(t.as_str().to_string()));
            }
        }
        all_params.push(Box::new(limit as i64));
        all_params.push(Box::new(offset as i64));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            all_params.iter().map(|p| p.as_ref()).collect();

        let entries = stmt
            .query_map(param_refs.as_slice(), row_to_entry)?
            .filter_map(|r| r.ok())
            .collect();

        Ok(entries)
    }

    fn search(&self, query: &MemorySearchQuery) -> Result<Vec<MemoryEntry>> {
        let conn = self.read_conn()?;
        let limit = query.limit.unwrap_or(20);

        // Load configurable search parameters
        let hybrid_cfg = load_hybrid_search_config();
        let decay_cfg = load_temporal_decay_config();

        // Try hybrid search (FTS5 + vector), fall back to FTS5-only
        let query_embedding = self.generate_embedding(&query.query);
        let has_vec = query_embedding.is_some();

        // ── Step 1: FTS5 keyword search (with query expansion) ──
        let fts_query = crate::memory::helpers::expand_query(&query.query);
        let mut fts_results: Vec<(i64, f64)> = Vec::new(); // (id, rank)

        {
            let mut stmt = conn.prepare(
                "SELECT fts.rowid, rank FROM memories_fts fts
                 WHERE memories_fts MATCH ?1
                 ORDER BY rank LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![fts_query, (limit * 3) as i64], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
            })?;
            for r in rows.flatten() {
                fts_results.push(r);
            }
        }

        // ── Step 2: Vector similarity search (if embedder available) ──
        let mut vec_results: Vec<(i64, f64)> = Vec::new(); // (id, distance)

        if let Some(ref emb) = query_embedding {
            let emb_bytes: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();
            if let Ok(mut stmt) = conn.prepare(
                "SELECT rowid, distance FROM memories_vec
                 WHERE embedding MATCH ?1
                 ORDER BY distance LIMIT ?2",
            ) {
                let rows = stmt.query_map(params![emb_bytes, (limit * 3) as i64], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
                });
                if let Ok(rows) = rows {
                    for r in rows.flatten() {
                        vec_results.push(r);
                    }
                }
            }
        }

        // ── Step 3: Weighted RRF (Reciprocal Rank Fusion) to merge results ──
        use std::collections::HashMap;
        let k = hybrid_cfg.rrf_k;

        let mut scores: HashMap<i64, f64> = HashMap::new();

        for (rank, (id, _)) in fts_results.iter().enumerate() {
            *scores.entry(*id).or_insert(0.0) +=
                hybrid_cfg.text_weight as f64 / (k + rank as f64 + 1.0);
        }

        if has_vec {
            for (rank, (id, _)) in vec_results.iter().enumerate() {
                *scores.entry(*id).or_insert(0.0) +=
                    hybrid_cfg.vector_weight as f64 / (k + rank as f64 + 1.0);
            }
        }

        // ── Step 3b: Apply temporal decay ──
        if decay_cfg.enabled && decay_cfg.half_life_days > 0.0 {
            let lambda = (2.0_f64).ln() / decay_cfg.half_life_days;
            let now = chrono::Utc::now();
            // Need to load updated_at for scored entries to apply decay
            let ids: Vec<i64> = scores.keys().cloned().collect();
            if !ids.is_empty() {
                let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                let sql = format!(
                    "SELECT id, updated_at, pinned FROM memories WHERE id IN ({})",
                    placeholders
                );
                let params: Vec<Box<dyn rusqlite::types::ToSql>> = ids
                    .iter()
                    .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
                    .collect();
                let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                    params.iter().map(|p| p.as_ref()).collect();
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(param_refs.as_slice(), |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, bool>(2)?,
                    ))
                })?;
                for r in rows.flatten() {
                    let (id, updated_at, pinned) = r;
                    if pinned {
                        continue;
                    } // Pinned memories are evergreen
                    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&updated_at) {
                        let age_days =
                            (now - dt.with_timezone(&chrono::Utc)).num_seconds() as f64 / 86400.0;
                        if age_days > 0.0 {
                            if let Some(score) = scores.get_mut(&id) {
                                *score *= (-lambda * age_days).exp();
                            }
                        }
                    }
                }
            }
        }

        // Sort by fused score (descending)
        let mut scored_ids: Vec<(i64, f64)> = scores.into_iter().collect();
        scored_ids.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored_ids.truncate(limit);

        if scored_ids.is_empty() {
            return Ok(Vec::new());
        }

        // ── Step 4: Load full entries for top results ──
        let id_list: Vec<String> = scored_ids.iter().map(|(id, _)| id.to_string()).collect();
        let placeholders = id_list.iter().map(|_| "?").collect::<Vec<_>>().join(",");

        // Apply scope and type filters
        let (scope_clause, mut scope_params) =
            scope_where(query.scope.as_ref(), query.agent_id.as_deref());
        let type_clause = if let Some(ref types) = query.types {
            if types.is_empty() {
                "1=1".to_string()
            } else {
                let ph: Vec<String> = types.iter().map(|_| "?".to_string()).collect();
                format!("memory_type IN ({})", ph.join(", "))
            }
        } else {
            "1=1".to_string()
        };

        let sql = format!(
            "SELECT id, memory_type, scope_type, scope_agent_id, content, tags,
                    source, source_session_id, pinned, created_at, updated_at
             FROM memories
             WHERE id IN ({}) AND {} AND {}",
            placeholders, scope_clause, type_clause
        );

        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        for (id, _) in &scored_ids {
            all_params.push(Box::new(*id));
        }
        all_params.append(&mut scope_params);
        if let Some(ref types) = query.types {
            for t in types {
                all_params.push(Box::new(t.as_str().to_string()));
            }
        }

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            all_params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql)?;

        let score_map: HashMap<i64, f64> = scored_ids.into_iter().collect();
        let mut entries: Vec<MemoryEntry> = stmt
            .query_map(param_refs.as_slice(), row_to_entry)?
            .filter_map(|r| r.ok())
            .map(|mut e| {
                e.relevance_score = score_map.get(&e.id).map(|s| *s as f32);
                e
            })
            .collect();

        // Sort by relevance score (descending)
        entries.sort_by(|a, b| {
            b.relevance_score
                .unwrap_or(0.0)
                .partial_cmp(&a.relevance_score.unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // ── Step 5: MMR diversity reranking ──
        let mmr_cfg = crate::memory::helpers::load_mmr_config();
        if mmr_cfg.enabled && entries.len() > 1 {
            let candidates: Vec<(i64, f32, &str)> = entries
                .iter()
                .map(|e| (e.id, e.relevance_score.unwrap_or(0.0), e.content.as_str()))
                .collect();
            let reranked = crate::memory::mmr::mmr_rerank(&candidates, limit, mmr_cfg.lambda);

            // Rebuild entries in MMR order
            let id_order: Vec<i64> = reranked.iter().map(|(id, _)| *id).collect();
            let entry_map: HashMap<i64, MemoryEntry> =
                entries.into_iter().map(|e| (e.id, e)).collect();
            entries = id_order
                .into_iter()
                .filter_map(|id| entry_map.get(&id).cloned())
                .collect();
        }

        Ok(entries)
    }

    fn count(&self, scope: Option<&MemoryScope>) -> Result<usize> {
        let conn = self.read_conn()?;
        let (scope_clause, scope_params) = scope_where(scope, None);

        let sql = format!("SELECT COUNT(*) FROM memories WHERE {}", scope_clause);
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            scope_params.iter().map(|p| p.as_ref()).collect();

        let count: i64 = conn.query_row(&sql, param_refs.as_slice(), |row| row.get(0))?;
        Ok(count as usize)
    }

    fn build_prompt_summary(&self, agent_id: &str, shared: bool, budget: usize) -> Result<String> {
        let candidates = self.load_prompt_candidates(agent_id, shared)?;
        Ok(format_prompt_summary(&candidates, budget))
    }

    /// Load candidate memories for prompt injection.
    /// Returns agent-scoped + optionally global memories, ordered by updated_at DESC.
    /// Used directly by `build_prompt_summary` and by LLM memory selection.
    fn load_prompt_candidates(&self, agent_id: &str, shared: bool) -> Result<Vec<MemoryEntry>> {
        let mut all_memories = Vec::new();

        // Agent-scoped memories
        let agent_scope = MemoryScope::Agent {
            id: agent_id.to_string(),
        };
        let agent_mems = self.list(Some(&agent_scope), None, 200, 0)?;
        all_memories.extend(agent_mems);

        // Global memories (if shared)
        if shared {
            let global_mems = self.list(Some(&MemoryScope::Global), None, 200, 0)?;
            all_memories.extend(global_mems);
        }

        Ok(all_memories)
    }

    fn export_markdown(&self, scope: Option<&MemoryScope>) -> Result<String> {
        let entries = self.list(scope, None, 10000, 0)?;

        if entries.is_empty() {
            return Ok("# Memories\n\nNo memories stored.\n".to_string());
        }

        let mut md = "# Memories\n\n".to_string();

        let type_order = [
            MemoryType::User,
            MemoryType::Feedback,
            MemoryType::Project,
            MemoryType::Reference,
        ];

        for mem_type in &type_order {
            let type_entries: Vec<&MemoryEntry> = entries
                .iter()
                .filter(|m| &m.memory_type == mem_type)
                .collect();

            if type_entries.is_empty() {
                continue;
            }

            md.push_str(&format!("## {}\n\n", mem_type.heading()));

            for entry in type_entries {
                md.push_str(&format!(
                    "### {}\n",
                    entry.content.lines().next().unwrap_or("Untitled")
                ));
                if !entry.tags.is_empty() {
                    md.push_str(&format!("Tags: {}\n", entry.tags.join(", ")));
                }
                let scope_label = match &entry.scope {
                    MemoryScope::Global => "global".to_string(),
                    MemoryScope::Agent { id } => format!("agent:{}", id),
                };
                md.push_str(&format!(
                    "Scope: {} | Source: {} | Updated: {}\n\n",
                    scope_label, entry.source, entry.updated_at
                ));
                md.push_str(&entry.content);
                md.push_str("\n\n---\n\n");
            }
        }

        Ok(md)
    }

    fn stats(&self, scope: Option<&MemoryScope>) -> Result<MemoryStats> {
        let conn = self.read_conn()?;
        let (scope_clause, scope_params) = scope_where(scope, None);

        // Total count
        let total: usize = conn.query_row(
            &format!("SELECT COUNT(*) FROM memories WHERE {}", scope_clause),
            rusqlite::params_from_iter(scope_params.iter()),
            |row| row.get::<_, i64>(0),
        )? as usize;

        // Count by type
        let mut by_type = std::collections::HashMap::new();
        {
            let (sc, sp) = scope_where(scope, None);
            let mut stmt = conn.prepare(&format!(
                "SELECT memory_type, COUNT(*) FROM memories WHERE {} GROUP BY memory_type",
                sc
            ))?;
            let rows = stmt.query_map(rusqlite::params_from_iter(sp.iter()), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
            })?;
            for row in rows {
                let (t, c) = row?;
                by_type.insert(t, c);
            }
        }

        // Count with embedding
        let with_embedding: usize = {
            let (sc, sp) = scope_where(scope, None);
            conn.query_row(
                &format!(
                    "SELECT COUNT(*) FROM memories WHERE {} AND id IN (SELECT rowid FROM memory_vec)",
                    sc
                ),
                rusqlite::params_from_iter(sp.iter()),
                |row| row.get::<_, i64>(0).map(|v| v as usize),
            ).unwrap_or(0)
        };

        // Oldest and newest
        let (oldest, newest) = {
            let (sc, sp) = scope_where(scope, None);
            let oldest: Option<String> = conn
                .query_row(
                    &format!("SELECT MIN(created_at) FROM memories WHERE {}", sc),
                    rusqlite::params_from_iter(sp.iter()),
                    |row| row.get(0),
                )
                .ok()
                .flatten();
            let (sc2, sp2) = scope_where(scope, None);
            let newest: Option<String> = conn
                .query_row(
                    &format!("SELECT MAX(created_at) FROM memories WHERE {}", sc2),
                    rusqlite::params_from_iter(sp2.iter()),
                    |row| row.get(0),
                )
                .ok()
                .flatten();
            (oldest, newest)
        };

        Ok(MemoryStats {
            total,
            by_type,
            with_embedding,
            oldest,
            newest,
        })
    }

    fn find_similar(
        &self,
        content: &str,
        memory_type: Option<&MemoryType>,
        scope: Option<&MemoryScope>,
        threshold: f32,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>> {
        // Reuse search() to get candidates via FTS5 + vector hybrid
        let types = memory_type.map(|t| vec![t.clone()]);
        let query = MemorySearchQuery {
            query: content.to_string(),
            types,
            scope: scope.cloned(),
            agent_id: None,
            limit: Some(limit * 3), // fetch extra to filter by threshold
        };
        let results = self.search(&query)?;

        // Filter by threshold
        Ok(results
            .into_iter()
            .filter(|e| e.relevance_score.unwrap_or(0.0) >= threshold)
            .take(limit)
            .collect())
    }

    fn add_with_dedup(
        &self,
        entry: NewMemory,
        threshold_high: f32,
        threshold_merge: f32,
    ) -> Result<AddResult> {
        // Find similar entries of the same type and scope
        let similar = self.find_similar(
            &entry.content,
            Some(&entry.memory_type),
            Some(&entry.scope),
            threshold_merge,
            5,
        )?;

        if let Some(best) = similar.first() {
            let score = best.relevance_score.unwrap_or(0.0);
            if score >= threshold_high {
                // Very similar — treat as duplicate, skip
                return Ok(AddResult::Duplicate {
                    existing_id: best.id,
                    score,
                });
            }
            // Moderately similar — update existing entry by appending new content
            let merged_content = format!("{}\n{}", best.content, entry.content);
            let mut merged_tags = best.tags.clone();
            for tag in &entry.tags {
                if !merged_tags.contains(tag) {
                    merged_tags.push(tag.clone());
                }
            }
            self.update(best.id, &merged_content, &merged_tags)?;
            return Ok(AddResult::Updated { id: best.id });
        }

        // No similar entries — create new
        let id = self.add(entry)?;
        Ok(AddResult::Created { id })
    }

    fn delete_batch(&self, ids: &[i64]) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let conn = self.write_conn()?;
        let placeholders: Vec<String> = ids.iter().map(|_| "?".to_string()).collect();
        let sql = format!(
            "DELETE FROM memories WHERE id IN ({})",
            placeholders.join(",")
        );
        let params: Vec<Box<dyn rusqlite::types::ToSql>> = ids
            .iter()
            .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
            .collect();
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let deleted = conn.execute(&sql, param_refs.as_slice())?;

        // Also clean vec0 table
        let dims = self
            .embedding_dims
            .load(std::sync::atomic::Ordering::Relaxed);
        if dims > 0 {
            let vec_sql = format!(
                "DELETE FROM memories_vec WHERE rowid IN ({})",
                placeholders.join(",")
            );
            let _ = conn.execute(&vec_sql, param_refs.as_slice());
        }

        Ok(deleted)
    }

    fn import_entries(&self, entries: Vec<NewMemory>, dedup: bool) -> Result<ImportResult> {
        let mut result = ImportResult {
            created: 0,
            skipped_duplicate: 0,
            failed: 0,
            errors: Vec::new(),
        };

        let dedup_cfg = load_dedup_config();
        for entry in entries {
            if dedup {
                match self.add_with_dedup(
                    entry,
                    dedup_cfg.threshold_high,
                    dedup_cfg.threshold_merge,
                ) {
                    Ok(AddResult::Created { .. }) => result.created += 1,
                    Ok(AddResult::Duplicate { .. }) => result.skipped_duplicate += 1,
                    Ok(AddResult::Updated { .. }) => result.created += 1, // count merge as created
                    Err(e) => {
                        result.failed += 1;
                        result.errors.push(e.to_string());
                    }
                }
            } else {
                match self.add(entry) {
                    Ok(_) => result.created += 1,
                    Err(e) => {
                        result.failed += 1;
                        result.errors.push(e.to_string());
                    }
                }
            }
        }

        Ok(result)
    }

    fn reembed_all(&self) -> Result<usize> {
        let entries = self.list(None, None, 100000, 0)?;
        self.reembed_entries(&entries)
    }

    fn reembed_batch(&self, ids: &[i64]) -> Result<usize> {
        let mut entries = Vec::new();
        for id in ids {
            if let Some(entry) = self.get(*id)? {
                entries.push(entry);
            }
        }
        self.reembed_entries(&entries)
    }

    fn set_embedder(&self, provider: Arc<dyn EmbeddingProvider>) {
        let dims = provider.dimensions();
        self.embedding_dims
            .store(dims, std::sync::atomic::Ordering::Relaxed);
        if let Ok(conn) = self.write_conn() {
            let _ = self.ensure_vec_table(&conn, dims);
        }
        *self.embedder.write().unwrap_or_else(|e| e.into_inner()) = Some(provider);
    }

    fn clear_embedder(&self) {
        *self.embedder.write().unwrap_or_else(|e| e.into_inner()) = None;
        self.embedding_dims
            .store(0, std::sync::atomic::Ordering::Relaxed);
    }

    fn has_embedder(&self) -> bool {
        self.embedder
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }
}

// ── Convenience: open default DB ────────────────────────────────

/// Open the default memory database at ~/.opencomputer/memory.db
#[allow(dead_code)]
pub fn open_default() -> Result<SqliteMemoryBackend> {
    let db_path = crate::paths::memory_db_path()?;
    SqliteMemoryBackend::open(&db_path)
}
