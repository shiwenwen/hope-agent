//! Backup/restore safety checks that need the process-global claim/dreaming/episode
//! stores. Keep this file to one test so the OnceLocks are initialised exactly
//! once in this integration-test process.

use std::sync::Arc;

use ha_core::memory::claims::{
    self, ClaimCandidate, ClaimDetail, ClaimLink, ClaimRecord, EvidenceRecord,
};
use ha_core::memory::dreaming::{self, ProfileSnapshotRecord};
use ha_core::memory::{
    MemoryBackend, MemoryBackupBundle, MemoryBackupManifest, MemoryBackupStructuredRestoreOptions,
    MemoryEpisodeRecord, MemoryExperienceHistoryQuery, MemoryExperienceHistoryRecord,
    MemoryProcedureRecord, MemoryScope, MemoryType, NewMemory, SqliteMemoryBackend,
};
use serde_json::json;

fn temp_backend() -> Arc<SqliteMemoryBackend> {
    let tmp = tempfile::tempdir().expect("tempdir");
    // Persist the tempdir for the lifetime of the process. The SQLite backend
    // owns pooled connections that outlive this helper's stack frame.
    let path = tmp.keep().join("memory.db");
    Arc::new(SqliteMemoryBackend::open(&path).expect("open backend"))
}

#[test]
fn structured_restore_downgrades_conflicts_and_maps_links_safely() {
    let target = temp_backend();
    claims::init_claim_store(target.clone());
    dreaming::init_store(target.clone());
    ha_core::memory::episodes::init_episode_store(target.clone());

    let source = temp_backend();
    let backup_memory_id = source
        .add(NewMemory {
            memory_type: MemoryType::User,
            scope: MemoryScope::Global,
            content: "The user prefers espresso.".to_string(),
            tags: vec!["drink".to_string()],
            source: "user".to_string(),
            source_session_id: None,
            pinned: false,
            attachment_path: None,
            attachment_mime: None,
        })
        .unwrap();
    let backup_memory = source.get(backup_memory_id).unwrap().unwrap();

    target
        .add(NewMemory {
            memory_type: MemoryType::Reference,
            scope: MemoryScope::Global,
            content: "Unrelated row to force a different local memory id.".to_string(),
            tags: vec![],
            source: "user".to_string(),
            source_session_id: None,
            pinned: false,
            attachment_path: None,
            attachment_mime: None,
        })
        .unwrap();
    let local_memory_id = target
        .add(NewMemory {
            memory_type: MemoryType::User,
            scope: MemoryScope::Global,
            content: "The user prefers espresso.".to_string(),
            tags: vec!["drink".to_string()],
            source: "user".to_string(),
            source_session_id: None,
            pinned: false,
            attachment_path: None,
            attachment_mime: None,
        })
        .unwrap();
    assert_ne!(backup_memory_id, local_memory_id);

    claims::write_claim_candidate(
        &ClaimCandidate {
            claim_type: "preference".to_string(),
            subject: "user".to_string(),
            predicate: "prefers".to_string(),
            object: "tea".to_string(),
            content: "The user prefers tea.".to_string(),
            scope: None,
            evidence_class: Some("explicit_user_statement".to_string()),
            salience: Some(0.7),
            temporal: None,
            evidence_refs: Vec::new(),
            tags: vec!["drink".to_string()],
        },
        &MemoryScope::Global,
        "target-session",
        None,
    )
    .unwrap();

    dreaming::insert_profile_snapshot_for_restore(
        "global",
        "",
        "- Existing local profile\n",
        "local-profile-run",
    )
    .unwrap();

    let now = "2026-01-01T00:00:00.000Z".to_string();
    let backup_claim = ClaimDetail {
        claim: ClaimRecord {
            id: "backup-claim-espresso".to_string(),
            scope_type: "global".to_string(),
            scope_id: None,
            claim_type: "preference".to_string(),
            subject: "user".to_string(),
            predicate: "prefers".to_string(),
            object: "espresso".to_string(),
            content: "The user prefers espresso.".to_string(),
            tags: vec!["drink".to_string()],
            confidence: 0.8,
            confidence_source: "derived".to_string(),
            salience: 0.8,
            freshness_policy: json!({}),
            status: "active".to_string(),
            valid_from: None,
            valid_until: None,
            supersedes_claim_id: None,
            source_run_id: Some("backup-run".to_string()),
            created_at: now.clone(),
            updated_at: now.clone(),
        },
        evidence: vec![EvidenceRecord {
            id: "backup-evidence-espresso".to_string(),
            claim_id: "backup-claim-espresso".to_string(),
            source_type: "memory".to_string(),
            evidence_class: "explicit_user_statement".to_string(),
            source_id: backup_memory_id.to_string(),
            session_id: None,
            message_id: None,
            file_path: None,
            url: None,
            quote: Some("The user prefers espresso.".to_string()),
            redaction_status: "raw_allowed".to_string(),
            access_scope: json!({}),
            weight: 1.0,
            created_at: now.clone(),
        }],
        links: vec![ClaimLink {
            claim_id: "backup-claim-espresso".to_string(),
            memory_id: backup_memory_id,
            sync_mode: "detached".to_string(),
            last_synced_claim_status: Some("active".to_string()),
            created_at: now.clone(),
            updated_at: now.clone(),
        }],
    };
    let backup_episode = MemoryEpisodeRecord {
        id: "backup-episode-release".to_string(),
        scope: MemoryScope::Global,
        title: "Release check recovered".to_string(),
        situation: "Release verification failed after packaging.".to_string(),
        actions: vec![
            "Inspected CI logs".to_string(),
            "Rebuilt package metadata".to_string(),
        ],
        outcome: "Release verification passed.".to_string(),
        lesson: "Check package metadata before retrying release verification.".to_string(),
        source_session_id: Some("backup-session".to_string()),
        source_message_ids: vec!["m1".to_string()],
        success_score: 0.9,
        tags: vec!["release".to_string()],
        status: "active".to_string(),
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    let backup_procedure = MemoryProcedureRecord {
        id: "backup-procedure-release".to_string(),
        scope: MemoryScope::Global,
        title: "Recover release verification".to_string(),
        trigger: "Release verification fails after packaging".to_string(),
        steps_markdown: "1. Inspect CI logs\n2. Rebuild package metadata".to_string(),
        constraints_markdown: "Keep local changes auditable.".to_string(),
        confidence: 0.9,
        status: "active".to_string(),
        source_episode_ids: vec![backup_episode.id.clone()],
        tags: vec!["release".to_string()],
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    let backup_episode_history = MemoryExperienceHistoryRecord {
        id: "backup-exp-history-episode".to_string(),
        target_kind: "episode".to_string(),
        target_id: backup_episode.id.clone(),
        action: "add".to_string(),
        scope: MemoryScope::Global,
        title_preview: "Release check recovered".to_string(),
        content_preview: "Episode was imported from the backup bundle.".to_string(),
        created_at: now.clone(),
    };
    let backup_procedure_history = MemoryExperienceHistoryRecord {
        id: "backup-exp-history-procedure".to_string(),
        target_kind: "procedure".to_string(),
        target_id: backup_procedure.id.clone(),
        action: "promote".to_string(),
        scope: MemoryScope::Global,
        title_preview: "Recover release verification".to_string(),
        content_preview: "Procedure was promoted before the backup was exported.".to_string(),
        created_at: now.clone(),
    };
    let unmapped_experience_history = MemoryExperienceHistoryRecord {
        id: "backup-exp-history-missing".to_string(),
        target_kind: "episode".to_string(),
        target_id: "missing-episode".to_string(),
        action: "update".to_string(),
        scope: MemoryScope::Global,
        title_preview: "Missing episode".to_string(),
        content_preview: "This audit row should remain unmapped.".to_string(),
        created_at: now.clone(),
    };

    let bundle = MemoryBackupBundle {
        schema_version: ha_core::memory::MEMORY_BACKUP_SCHEMA_VERSION.to_string(),
        exported_at: now.clone(),
        app_version: "test".to_string(),
        manifest: MemoryBackupManifest {
            complete: true,
            legacy_memory_count: 1,
            legacy_history_count: 0,
            attachment_ref_count: 0,
            attachment_payload_count: 0,
            attachment_chunk_count: 0,
            attachment_chunked_ref_count: 0,
            attachment_external_ref_count: 0,
            attachment_payload_bytes: 0,
            attachment_missing_count: 0,
            claim_count: 1,
            evidence_count: 1,
            claim_link_count: 1,
            profile_snapshot_count: 1,
            episode_count: 1,
            procedure_count: 1,
            experience_history_count: 3,
            unsupported_sections: Vec::new(),
            warnings: Vec::new(),
        },
        stats: source.stats(None).unwrap(),
        health: None,
        config_manifest: json!({}),
        legacy_memories: vec![backup_memory],
        legacy_history: Vec::new(),
        attachment_payloads: Vec::new(),
        attachment_payload_chunks: Vec::new(),
        attachment_external_payloads: Vec::new(),
        legacy_markdown: source.export_markdown(None).unwrap(),
        claims: vec![backup_claim],
        profile_snapshots: vec![ProfileSnapshotRecord {
            scope_type: "global".to_string(),
            scope_id: None,
            version: 1,
            body_md: "- Imported profile should not silently replace local profile\n".to_string(),
            sources: Vec::new(),
            source_run_id: "backup-profile-run".to_string(),
            created_at: now.clone(),
        }],
        episodes: vec![backup_episode],
        procedures: vec![backup_procedure],
        experience_history: vec![
            backup_episode_history,
            backup_procedure_history,
            unmapped_experience_history,
        ],
    };

    let content = serde_json::to_string(&bundle).unwrap();
    let preview = ha_core::memory::preview_backup_bundle(target.as_ref(), &content).unwrap();
    assert!(preview.valid);
    assert_eq!(preview.claim_restore_plan.import_candidates, 1);
    assert_eq!(preview.claim_restore_plan.conflicting_candidates, 1);
    assert_eq!(preview.claim_restore_plan.conflict_examples.len(), 1);
    let conflict = &preview.claim_restore_plan.conflict_examples[0];
    assert_eq!(conflict.incoming_claim_id, "backup-claim-espresso");
    assert_eq!(conflict.incoming_object, "espresso");
    assert_eq!(conflict.existing_object, "tea");
    assert_eq!(preview.profile_restore_plan.import_candidates, 1);
    assert_eq!(preview.profile_restore_plan.conflicting_scope_candidates, 1);
    assert_eq!(preview.episode_count, 1);
    assert_eq!(preview.episode_import_candidates, 1);
    assert_eq!(preview.procedure_count, 1);
    assert_eq!(preview.procedure_import_candidates, 1);
    assert_eq!(preview.experience_history_count, 3);
    assert_eq!(preview.experience_history_restorable, 2);
    assert_eq!(preview.experience_history_skipped_unmapped, 1);

    let result = ha_core::memory::restore_backup_structured_memory(
        target.as_ref(),
        &content,
        MemoryBackupStructuredRestoreOptions::default(),
    )
    .unwrap();

    assert_eq!(result.restored_claims, 1);
    assert_eq!(result.restored_claims_needing_review, 1);
    assert_eq!(result.restored_evidence_rows, 1);
    assert_eq!(result.restored_claim_links, 1);
    assert_eq!(result.skipped_claim_links, 0);
    assert_eq!(result.restored_profile_snapshots, 0);
    assert_eq!(result.skipped_profile_scope_conflicts, 1);
    assert_eq!(result.restored_episodes, 1);
    assert_eq!(result.restored_procedures, 1);
    assert_eq!(result.restored_experience_history, 2);
    assert_eq!(result.skipped_experience_history_unmapped, 1);
    assert_eq!(result.preview.episode_exact_matches, 1);
    assert_eq!(result.preview.procedure_exact_matches, 1);

    let restored = claims::get_claim("backup-claim-espresso")
        .unwrap()
        .expect("backup claim restored");
    assert_eq!(restored.claim.status, "needs_review");
    assert_eq!(restored.evidence.len(), 1);
    assert_eq!(restored.links.len(), 1);
    assert_eq!(restored.links[0].memory_id, local_memory_id);

    let profiles = dreaming::list_profile_snapshots().unwrap();
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].body_md, "- Existing local profile\n");
    let restored_episode = ha_core::memory::episodes::get_episode("backup-episode-release")
        .unwrap()
        .expect("backup episode restored");
    assert_eq!(
        restored_episode.lesson,
        "Check package metadata before retrying release verification."
    );
    let restored_procedure = ha_core::memory::episodes::get_procedure("backup-procedure-release")
        .unwrap()
        .expect("backup procedure restored");
    assert_eq!(
        restored_procedure.source_episode_ids,
        vec!["backup-episode-release".to_string()]
    );
    let restored_episode_history =
        ha_core::memory::episodes::list_experience_history_page(MemoryExperienceHistoryQuery {
            target_kind: Some("episode".to_string()),
            target_id: Some("backup-episode-release".to_string()),
            limit: Some(10),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(restored_episode_history.total, 2);
    assert!(restored_episode_history
        .items
        .iter()
        .any(|item| item.id == "backup-exp-history-episode"));
    let restored_procedure_history =
        ha_core::memory::episodes::list_experience_history_page(MemoryExperienceHistoryQuery {
            target_kind: Some("procedure".to_string()),
            target_id: Some("backup-procedure-release".to_string()),
            limit: Some(10),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(restored_procedure_history.total, 2);
    assert!(restored_procedure_history
        .items
        .iter()
        .any(|item| item.id == "backup-exp-history-procedure"));

    let result_with_profile_override = ha_core::memory::restore_backup_structured_memory(
        target.as_ref(),
        &content,
        MemoryBackupStructuredRestoreOptions {
            allow_profile_scope_conflicts: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(result_with_profile_override.restored_claims, 0);
    assert_eq!(result_with_profile_override.restored_profile_snapshots, 1);
    assert_eq!(result_with_profile_override.restored_episodes, 0);
    assert_eq!(result_with_profile_override.restored_procedures, 0);
    assert_eq!(result_with_profile_override.restored_experience_history, 0);
    assert_eq!(
        result_with_profile_override.skipped_experience_history_unmapped,
        1
    );
    assert_eq!(
        result_with_profile_override.skipped_profile_scope_conflicts,
        0
    );

    let profiles = dreaming::list_profile_snapshots().unwrap();
    assert_eq!(profiles.len(), 1);
    assert_eq!(
        profiles[0].body_md,
        "- Imported profile should not silently replace local profile\n"
    );
}
