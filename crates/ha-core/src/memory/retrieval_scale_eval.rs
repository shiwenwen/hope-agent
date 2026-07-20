//! Deterministic realistic-scale retrieval capability eval.
//!
//! Compiled only for the standalone `hope-agent-eval` runner. Retrieval quality
//! is blocking; latency is reported as advisory evidence.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::{save_config, AppConfig};
use crate::memory::claims;
use crate::memory::{
    EmbeddingProvider, EmbeddingSelection, MemoryBackend, MemorySearchQuery, SqliteMemoryBackend,
};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const DEFAULT_ROWS: usize = 50_000;
const DEFAULT_QUERIES_PER_CLASS: usize = 24;
pub const DEFAULT_P95_SLO_MS: f64 = 250.0;
const BENCHMARK_EMBEDDING_DIMS: u32 = 8;
const BENCHMARK_EMBEDDING_SIGNATURE: &str = "memory-scale-benchmark-v1";

struct BenchmarkEmbedder;

impl EmbeddingProvider for BenchmarkEmbedder {
    fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        Ok(benchmark_embedding(text))
    }

    fn embed_batch(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|text| benchmark_embedding(text)).collect())
    }

    fn dimensions(&self) -> u32 {
        BENCHMARK_EMBEDDING_DIMS
    }
}

fn benchmark_embedding(text: &str) -> Vec<f32> {
    let normalized = text.to_lowercase();
    let mut vector = vec![0.0f32; BENCHMARK_EMBEDDING_DIMS as usize];
    let dimensions: [(&[&str], usize); 7] = [
        (
            &["release", "deployment", "rollback", "ship", "发布", "部署"],
            0,
        ),
        (&["preference", "concise", "brief", "偏好", "简洁"], 1),
        (
            &[
                "database",
                "migration",
                "schema",
                "upgrade",
                "数据库",
                "迁移",
            ],
            2,
        ),
        (
            &[
                "frontend",
                "keyboard",
                "navigation",
                "accessibility",
                "键盘",
            ],
            3,
        ),
        (
            &[
                "provider",
                "authentication",
                "retry",
                "auth",
                "认证",
                "重试",
            ],
            4,
        ),
        (
            &[
                "architecture",
                "module",
                "ownership",
                "component",
                "架构",
                "模块",
            ],
            5,
        ),
        (
            &["meeting", "decision", "follow-up", "action", "会议", "决定"],
            6,
        ),
    ];
    for (needles, dimension) in dimensions {
        if needles.iter().any(|needle| normalized.contains(needle)) {
            vector[dimension] = 1.0;
        }
    }
    if vector.iter().all(|value| *value == 0.0) {
        vector[7] = 1.0;
    }
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    for value in &mut vector {
        *value /= norm.max(f32::EPSILON);
    }
    vector
}

fn embedding_bytes(vector: &[f32]) -> Vec<u8> {
    vector
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn env_usize(name: &str, default: usize, min: usize, max: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default)
        .clamp(min, max)
}

fn seed_realistic_memories(path: &Path, rows: usize) {
    drop(SqliteMemoryBackend::open(path).expect("create benchmark schema"));
    let mut connection = Connection::open(path).expect("open benchmark database");
    let transaction = connection.transaction().expect("start seed transaction");
    {
        let mut insert = transaction
            .prepare_cached(
                "INSERT INTO memories (
                    memory_type, scope_type, scope_agent_id, scope_project_id,
                    content, tags, source, source_session_id, pinned,
                    created_at, updated_at
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .expect("prepare seed insert");
        for index in 0..rows {
            let (scope_type, agent_id, project_id) = match index % 5 {
                0 | 1 => ("project", None, Some(format!("project-{}", index % 37))),
                2 | 3 => ("agent", Some(format!("agent-{}", index % 11)), None),
                _ => ("global", None, None),
            };
            let topic = match index % 7 {
                0 => "release incident and deployment rollback",
                1 => "user preference for concise technical answers",
                2 => "database migration and schema compatibility",
                3 => "frontend accessibility and keyboard navigation",
                4 => "provider authentication and retry policy",
                5 => "project architecture and module ownership",
                _ => "meeting decision and follow-up action",
            };
            let content = format!(
                "Memory record {index}: {topic}. Stable token memkey_{index:06}. 中文记忆编号{index:06}。"
            );
            let tags = format!(r#"["topic-{}","bucket-{}"]"#, index % 7, index % 97);
            let timestamp = format!("2026-06-{:02}T12:00:00.000Z", index % 28 + 1);
            insert
                .execute(params![
                    "user",
                    scope_type,
                    agent_id,
                    project_id,
                    content,
                    tags,
                    if index % 9 == 0 { "import" } else { "auto" },
                    format!("bench-session-{}", index % 251),
                    index % 101 == 0,
                    timestamp,
                    timestamp,
                ])
                .expect("insert benchmark row");
        }
    }
    transaction.commit().expect("commit benchmark seed");
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .expect("finalize benchmark database");
}

fn seed_realistic_claims(path: &Path, rows: usize) {
    let mut connection = Connection::open(path).expect("open claim benchmark database");
    let transaction = connection
        .transaction()
        .expect("start claim seed transaction");
    {
        let mut insert = transaction
            .prepare_cached(
                "INSERT INTO memory_claims (
                    id, scope_type, scope_id, claim_type, subject, predicate, object,
                    content, tags_json, confidence, confidence_source, salience,
                    freshness_policy_json, status, created_at, updated_at
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .expect("prepare claim seed insert");
        for index in 0..rows {
            let (scope_type, scope_id) = match index % 5 {
                0 | 1 => ("project", Some(format!("project-{}", index % 37))),
                2 | 3 => ("agent", Some(format!("agent-{}", index % 11))),
                _ => ("global", None),
            };
            let topic = match index % 7 {
                0 => "deployment rollback",
                1 => "concise answer preference",
                2 => "database migration",
                3 => "keyboard navigation",
                4 => "provider retry policy",
                5 => "module ownership",
                _ => "meeting decision",
            };
            let content = format!(
                "Structured memory {index}: {topic}. Stable token claimkey_{index:06}. 结构记忆编号{index:06}。"
            );
            let timestamp = format!("2026-06-{:02}T12:00:00.000Z", index % 28 + 1);
            insert
                .execute(params![
                    format!("bench-claim-{index:08}"),
                    scope_type,
                    scope_id,
                    if index % 3 == 0 { "preference" } else { "fact" },
                    "user",
                    format!("benchmark_predicate_{}", index % 19),
                    topic,
                    content,
                    format!(r#"["claim-topic-{}","bucket-{}"]"#, index % 7, index % 97),
                    0.55 + (index % 40) as f64 / 100.0,
                    "derived",
                    0.50 + (index % 45) as f64 / 100.0,
                    "{}",
                    "active",
                    timestamp,
                    timestamp,
                ])
                .expect("insert benchmark claim");
        }
    }
    transaction.commit().expect("commit benchmark claim seed");
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .expect("finalize claim benchmark database");
}

fn configure_benchmark_embedding() {
    let config = AppConfig {
        memory_embedding: EmbeddingSelection {
            enabled: true,
            model_config_id: None,
            active_signature: Some(BENCHMARK_EMBEDDING_SIGNATURE.to_string()),
            last_reembedded_signature: Some(BENCHMARK_EMBEDDING_SIGNATURE.to_string()),
        },
        ..Default::default()
    };
    save_config(&config).expect("persist isolated benchmark config");
}

fn seed_benchmark_vectors(path: &Path, rows: usize) {
    let mut connection = Connection::open(path).expect("open vector benchmark database");
    connection
        .execute_batch(
            "DROP TABLE IF EXISTS memories_vec;
             DROP TABLE IF EXISTS memory_claims_vec;
             CREATE VIRTUAL TABLE memories_vec USING vec0(
                 rowid INTEGER PRIMARY KEY,
                 embedding float[8]
             );
             CREATE VIRTUAL TABLE memory_claims_vec USING vec0(
                 rowid INTEGER PRIMARY KEY,
                 embedding float[8]
             );",
        )
        .expect("create benchmark vector indexes");
    let transaction = connection
        .transaction()
        .expect("start vector seed transaction");
    transaction
        .execute(
            "UPDATE memories SET embedding_signature = ?1",
            [BENCHMARK_EMBEDDING_SIGNATURE],
        )
        .expect("mark memory embedding signature");
    transaction
        .execute(
            "UPDATE memory_claims SET embedding_signature = ?1",
            [BENCHMARK_EMBEDDING_SIGNATURE],
        )
        .expect("mark claim embedding signature");
    {
        let mut insert_memory = transaction
            .prepare_cached("INSERT INTO memories_vec(rowid, embedding) VALUES (?1, ?2)")
            .expect("prepare memory vector insert");
        let mut insert_claim = transaction
            .prepare_cached("INSERT INTO memory_claims_vec(rowid, embedding) VALUES (?1, ?2)")
            .expect("prepare claim vector insert");
        let category_vectors = [
            benchmark_embedding("release deployment rollback"),
            benchmark_embedding("concise answer preference"),
            benchmark_embedding("database schema migration"),
            benchmark_embedding("frontend keyboard accessibility"),
            benchmark_embedding("provider authentication retry"),
            benchmark_embedding("architecture module ownership"),
            benchmark_embedding("meeting decision follow-up"),
        ]
        .map(|vector| embedding_bytes(&vector));
        for index in 0..rows {
            let rowid = index as i64 + 1;
            let vector = &category_vectors[index % category_vectors.len()];
            insert_memory
                .execute(params![rowid, vector])
                .expect("insert memory benchmark vector");
            insert_claim
                .execute(params![rowid, vector])
                .expect("insert claim benchmark vector");
        }
    }
    transaction.commit().expect("commit benchmark vectors");
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .expect("finalize vector benchmark database");
}

fn probe_claim_vector_fast_path(path: &Path) -> (Duration, usize) {
    let connection = Connection::open(path).expect("open claim vector probe database");
    let embedding = embedding_bytes(&benchmark_embedding("ship failure"));
    let started = Instant::now();
    let mut stmt = connection
        .prepare(
            "WITH nearest AS (
                SELECT rowid, distance FROM memory_claims_vec
                WHERE embedding MATCH ?1
                ORDER BY distance LIMIT ?2
             )
             SELECT nearest.rowid
             FROM nearest
             JOIN memory_claims c ON c.rowid = nearest.rowid
             WHERE c.embedding_signature = ?3
               AND c.status = 'active'
               AND (c.valid_until IS NULL OR c.valid_until = '' OR c.valid_until >= ?4)
             ORDER BY nearest.distance LIMIT ?5",
        )
        .expect("prepare claim vector fast-path probe");
    let rows = stmt
        .query_map(
            params![
                embedding,
                240i64,
                BENCHMARK_EMBEDDING_SIGNATURE,
                "2026-07-10T00:00:00.000Z",
                30i64,
            ],
            |row| row.get::<_, i64>(0),
        )
        .expect("query claim vector fast-path probe")
        .filter_map(|row| row.ok())
        .count();
    (started.elapsed(), rows)
}

fn search(backend: &SqliteMemoryBackend, query: &str, limit: usize) -> (Duration, Vec<String>) {
    let started = Instant::now();
    let results = backend
        .search(&MemorySearchQuery {
            query: query.to_string(),
            types: None,
            sources: None,
            scope: None,
            agent_id: None,
            limit: Some(limit),
        })
        .expect("benchmark search");
    (
        started.elapsed(),
        results.into_iter().map(|entry| entry.content).collect(),
    )
}

fn search_claims(query: &str, limit: usize) -> (Duration, Vec<String>) {
    let started = Instant::now();
    let results = claims::search_claims(query, None, limit).expect("benchmark claim search");
    (
        started.elapsed(),
        results.into_iter().map(|claim| claim.content).collect(),
    )
}

fn percentile_ms(samples: &mut [Duration], percentile: f64) -> f64 {
    samples.sort_unstable();
    if samples.is_empty() {
        return 0.0;
    }
    let index = ((samples.len() - 1) as f64 * percentile).round() as usize;
    samples[index].as_secs_f64() * 1_000.0
}

fn database_size_bytes(path: &Path) -> u64 {
    std::fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
        + std::fs::metadata(path.with_extension("db-wal"))
            .map(|metadata| metadata.len())
            .unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalScaleEvalReport {
    pub passed: bool,
    pub rows_per_store: usize,
    pub queries_per_class: usize,
    pub seed_ms: f64,
    pub database_bytes: u64,
    pub quality: BTreeMap<String, f64>,
    pub latency_ms: BTreeMap<String, f64>,
    pub failures: Vec<String>,
}

pub fn run_retrieval_scale_eval() -> RetrievalScaleEvalReport {
    let rows = env_usize("HA_MEMORY_BENCH_ROWS", DEFAULT_ROWS, 1_000, 250_000);
    let queries_per_class = env_usize("HA_MEMORY_BENCH_QUERIES", DEFAULT_QUERIES_PER_CLASS, 4, 200);
    let temp = tempfile::tempdir().expect("benchmark tempdir");
    let path = temp.path().join("memory.db");
    // Isolate this process from the developer's real embedding/MMR/decay
    // settings before the global config cache is first touched.
    let runtime_dir = temp.path().join("runtime");
    std::fs::create_dir_all(&runtime_dir).expect("create benchmark runtime dir");
    std::env::set_var("HA_DATA_DIR", &runtime_dir);
    configure_benchmark_embedding();

    let seed_started = Instant::now();
    seed_realistic_memories(&path, rows);
    seed_realistic_claims(&path, rows);
    seed_benchmark_vectors(&path, rows);
    let (claim_vector_probe_latency, claim_vector_probe_rows) = probe_claim_vector_fast_path(&path);
    let seed_ms = seed_started.elapsed().as_secs_f64() * 1_000.0;
    let backend = Arc::new(SqliteMemoryBackend::open(&path).expect("open seeded backend"));
    backend.set_embedder(Arc::new(BenchmarkEmbedder));
    claims::init_claim_store(backend.clone());

    let _ = search(&backend, "release incident", 10);
    let _ = search_claims("deployment rollback", 10);
    let mut exact_latencies = Vec::with_capacity(queries_per_class);
    let mut cjk_latencies = Vec::with_capacity(queries_per_class);
    let mut common_latencies = Vec::with_capacity(queries_per_class);
    let mut claim_exact_latencies = Vec::with_capacity(queries_per_class);
    let mut claim_cjk_latencies = Vec::with_capacity(queries_per_class);
    let mut claim_common_latencies = Vec::with_capacity(queries_per_class);
    let mut semantic_latencies = Vec::with_capacity(queries_per_class);
    let mut claim_semantic_latencies = Vec::with_capacity(queries_per_class);
    let mut exact_hits = 0usize;
    let mut cjk_hits = 0usize;
    let mut claim_exact_hits = 0usize;
    let mut claim_cjk_hits = 0usize;
    let mut semantic_hits = 0usize;
    let mut semantic_total = 0usize;
    let mut claim_semantic_hits = 0usize;
    let mut claim_semantic_total = 0usize;
    let semantic_cases = [
        ("ship failure", "release incident", "deployment rollback"),
        (
            "brief response style",
            "user preference",
            "concise answer preference",
        ),
        ("schema upgrade", "database migration", "database migration"),
        (
            "keyboard access",
            "keyboard navigation",
            "keyboard navigation",
        ),
        (
            "auth retry",
            "provider authentication",
            "provider retry policy",
        ),
        ("component owner", "module ownership", "module ownership"),
        ("follow-up result", "meeting decision", "meeting decision"),
    ];

    for iteration in 0..queries_per_class {
        let index = (iteration.saturating_mul(7_919).saturating_add(17)) % rows;
        let exact = format!("memkey_{index:06}");
        let (elapsed, results) = search(&backend, &exact, 10);
        exact_latencies.push(elapsed);
        exact_hits += usize::from(results.iter().any(|content| content.contains(&exact)));

        let cjk = format!("编号{index:06}");
        let (elapsed, results) = search(&backend, &cjk, 10);
        cjk_latencies.push(elapsed);
        cjk_hits += usize::from(results.iter().any(|content| content.contains(&cjk)));

        let (elapsed, _) = search(&backend, "release incident", 10);
        common_latencies.push(elapsed);

        let claim_exact = format!("claimkey_{index:06}");
        let (elapsed, results) = search_claims(&claim_exact, 10);
        claim_exact_latencies.push(elapsed);
        claim_exact_hits +=
            usize::from(results.iter().any(|content| content.contains(&claim_exact)));

        let claim_cjk = format!("记忆编号{index:06}");
        let (elapsed, results) = search_claims(&claim_cjk, 10);
        claim_cjk_latencies.push(elapsed);
        claim_cjk_hits += usize::from(results.iter().any(|content| content.contains(&claim_cjk)));

        let (elapsed, _) = search_claims("deployment rollback", 10);
        claim_common_latencies.push(elapsed);

        let (semantic_query, legacy_marker, claim_marker) =
            semantic_cases[iteration % semantic_cases.len()];
        let (elapsed, results) = search(&backend, semantic_query, 10);
        semantic_latencies.push(elapsed);
        semantic_total += results.len();
        semantic_hits += results
            .iter()
            .filter(|content| content.contains(legacy_marker))
            .count();

        let (elapsed, results) = search_claims(semantic_query, 10);
        claim_semantic_latencies.push(elapsed);
        claim_semantic_total += results.len();
        claim_semantic_hits += results
            .iter()
            .filter(|content| content.contains(claim_marker))
            .count();
    }

    let exact_recall = exact_hits as f64 / queries_per_class as f64;
    let cjk_recall = cjk_hits as f64 / queries_per_class as f64;
    let claim_exact_recall = claim_exact_hits as f64 / queries_per_class as f64;
    let claim_cjk_recall = claim_cjk_hits as f64 / queries_per_class as f64;
    let semantic_precision = semantic_hits as f64 / semantic_total.max(1) as f64;
    let claim_semantic_precision = claim_semantic_hits as f64 / claim_semantic_total.max(1) as f64;
    let exact_p50_ms = percentile_ms(&mut exact_latencies, 0.50);
    let exact_p95_ms = percentile_ms(&mut exact_latencies, 0.95);
    let cjk_p50_ms = percentile_ms(&mut cjk_latencies, 0.50);
    let cjk_p95_ms = percentile_ms(&mut cjk_latencies, 0.95);
    let common_p50_ms = percentile_ms(&mut common_latencies, 0.50);
    let common_p95_ms = percentile_ms(&mut common_latencies, 0.95);
    let claim_exact_p50_ms = percentile_ms(&mut claim_exact_latencies, 0.50);
    let claim_exact_p95_ms = percentile_ms(&mut claim_exact_latencies, 0.95);
    let claim_cjk_p50_ms = percentile_ms(&mut claim_cjk_latencies, 0.50);
    let claim_cjk_p95_ms = percentile_ms(&mut claim_cjk_latencies, 0.95);
    let claim_common_p50_ms = percentile_ms(&mut claim_common_latencies, 0.50);
    let claim_common_p95_ms = percentile_ms(&mut claim_common_latencies, 0.95);
    let semantic_p50_ms = percentile_ms(&mut semantic_latencies, 0.50);
    let semantic_p95_ms = percentile_ms(&mut semantic_latencies, 0.95);
    let claim_semantic_p50_ms = percentile_ms(&mut claim_semantic_latencies, 0.50);
    let claim_semantic_p95_ms = percentile_ms(&mut claim_semantic_latencies, 0.95);

    println!(
        "MEMORY_SCALE_BENCH {{\"rowsPerStore\":{rows},\"queriesPerClass\":{queries_per_class},\"seedMs\":{seed_ms:.2},\"dbBytes\":{},\"claimVectorProbeRows\":{claim_vector_probe_rows},\"claimVectorProbeMs\":{:.3},\"legacyExactRecallAt10\":{exact_recall:.4},\"legacyCjkRecallAt10\":{cjk_recall:.4},\"legacySemanticPrecisionAt10\":{semantic_precision:.4},\"legacyExactP50Ms\":{exact_p50_ms:.3},\"legacyExactP95Ms\":{exact_p95_ms:.3},\"legacyCjkP50Ms\":{cjk_p50_ms:.3},\"legacyCjkP95Ms\":{cjk_p95_ms:.3},\"legacyCommonP50Ms\":{common_p50_ms:.3},\"legacyCommonP95Ms\":{common_p95_ms:.3},\"legacySemanticP50Ms\":{semantic_p50_ms:.3},\"legacySemanticP95Ms\":{semantic_p95_ms:.3},\"claimExactRecallAt10\":{claim_exact_recall:.4},\"claimCjkRecallAt10\":{claim_cjk_recall:.4},\"claimSemanticPrecisionAt10\":{claim_semantic_precision:.4},\"claimExactP50Ms\":{claim_exact_p50_ms:.3},\"claimExactP95Ms\":{claim_exact_p95_ms:.3},\"claimCjkP50Ms\":{claim_cjk_p50_ms:.3},\"claimCjkP95Ms\":{claim_cjk_p95_ms:.3},\"claimCommonP50Ms\":{claim_common_p50_ms:.3},\"claimCommonP95Ms\":{claim_common_p95_ms:.3},\"claimSemanticP50Ms\":{claim_semantic_p50_ms:.3},\"claimSemanticP95Ms\":{claim_semantic_p95_ms:.3}}}",
        database_size_bytes(&path),
        claim_vector_probe_latency.as_secs_f64() * 1_000.0,
    );

    let quality = BTreeMap::from([
        (
            "claimVectorProbeRows".to_string(),
            claim_vector_probe_rows as f64,
        ),
        ("legacyExactRecallAt10".to_string(), exact_recall),
        ("legacyCjkRecallAt10".to_string(), cjk_recall),
        (
            "legacySemanticPrecisionAt10".to_string(),
            semantic_precision,
        ),
        ("claimExactRecallAt10".to_string(), claim_exact_recall),
        ("claimCjkRecallAt10".to_string(), claim_cjk_recall),
        (
            "claimSemanticPrecisionAt10".to_string(),
            claim_semantic_precision,
        ),
    ]);
    let latency_ms = BTreeMap::from([
        (
            "claimVectorProbe".to_string(),
            claim_vector_probe_latency.as_secs_f64() * 1_000.0,
        ),
        ("legacyExactP50".to_string(), exact_p50_ms),
        ("legacyExactP95".to_string(), exact_p95_ms),
        ("legacyCjkP50".to_string(), cjk_p50_ms),
        ("legacyCjkP95".to_string(), cjk_p95_ms),
        ("legacyCommonP50".to_string(), common_p50_ms),
        ("legacyCommonP95".to_string(), common_p95_ms),
        ("legacySemanticP50".to_string(), semantic_p50_ms),
        ("legacySemanticP95".to_string(), semantic_p95_ms),
        ("claimExactP50".to_string(), claim_exact_p50_ms),
        ("claimExactP95".to_string(), claim_exact_p95_ms),
        ("claimCjkP50".to_string(), claim_cjk_p50_ms),
        ("claimCjkP95".to_string(), claim_cjk_p95_ms),
        ("claimCommonP50".to_string(), claim_common_p50_ms),
        ("claimCommonP95".to_string(), claim_common_p95_ms),
        ("claimSemanticP50".to_string(), claim_semantic_p50_ms),
        ("claimSemanticP95".to_string(), claim_semantic_p95_ms),
    ]);
    let mut failures = Vec::new();
    for (name, value, minimum) in [
        ("claimVectorProbeRows", claim_vector_probe_rows as f64, 8.0),
        ("legacyExactRecallAt10", exact_recall, 0.99),
        ("legacyCjkRecallAt10", cjk_recall, 0.99),
        ("legacySemanticPrecisionAt10", semantic_precision, 0.99),
        ("claimExactRecallAt10", claim_exact_recall, 0.99),
        ("claimCjkRecallAt10", claim_cjk_recall, 0.99),
        ("claimSemanticPrecisionAt10", claim_semantic_precision, 0.99),
    ] {
        if value < minimum {
            failures.push(format!("{name} {value:.4} is below {minimum:.4}"));
        }
    }
    RetrievalScaleEvalReport {
        passed: failures.is_empty(),
        rows_per_store: rows,
        queries_per_class,
        seed_ms,
        database_bytes: database_size_bytes(&path),
        quality,
        latency_ms,
        failures,
    }
}
