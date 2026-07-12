use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::memory::dreaming::{self, DreamingDecisionListFilter, DreamingDecisionListItem};
use crate::memory::{
    list_experience_history_page, MemoryExperienceHistoryListPage, MemoryExperienceHistoryQuery,
    MemoryExperienceHistoryRecord, MemoryHistoryAction, MemoryHistoryListResponse,
    MemoryHistoryQuery, MemoryHistoryRecord,
};

const DEFAULT_AUDIT_PAGE_SIZE: usize = 50;
const MAX_AUDIT_PAGE_SIZE: usize = 200;
const MAX_AUDIT_WINDOW: usize = 5_000;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryAuditPageQuery {
    #[serde(default)]
    pub query: Option<String>,
    /// `"all"` or a legacy memory action (`add`, `update`, `delete`, `pin`,
    /// `unpin`, `import`). Specific actions intentionally stay legacy-only so
    /// workflow and claim decisions are not forced into the legacy taxonomy.
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryAuditSource {
    ClaimDecision,
    Experience,
    LegacyMemory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "record")]
pub enum MemoryAuditRecord {
    LegacyMemory(MemoryHistoryRecord),
    Experience(MemoryExperienceHistoryRecord),
    ClaimDecision(DreamingDecisionListItem),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryAuditItem {
    pub source: MemoryAuditSource,
    pub id: String,
    pub created_at: String,
    pub item: MemoryAuditRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MemoryAuditSourceSummary {
    pub included: bool,
    pub total: usize,
    #[serde(default)]
    pub total_truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MemoryAuditSourceSummaries {
    pub legacy_memory: MemoryAuditSourceSummary,
    pub experience: MemoryAuditSourceSummary,
    pub claim_decision: MemoryAuditSourceSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryAuditPageResponse {
    pub items: Vec<MemoryAuditItem>,
    pub total: usize,
    #[serde(default)]
    pub total_truncated: bool,
    pub sources: MemoryAuditSourceSummaries,
}

fn parse_legacy_action(action: &str) -> Result<MemoryHistoryAction> {
    match action {
        "add" => Ok(MemoryHistoryAction::Add),
        "update" => Ok(MemoryHistoryAction::Update),
        "delete" => Ok(MemoryHistoryAction::Delete),
        "pin" => Ok(MemoryHistoryAction::Pin),
        "unpin" => Ok(MemoryHistoryAction::Unpin),
        "import" => Ok(MemoryHistoryAction::Import),
        other => Err(anyhow!("invalid memory audit action: {other}")),
    }
}

fn normalized_action(action: Option<&str>) -> Result<Option<MemoryHistoryAction>> {
    let Some(action) = action.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if action == "all" {
        return Ok(None);
    }
    parse_legacy_action(action).map(Some)
}

fn clamped_limit(limit: Option<usize>) -> usize {
    limit
        .unwrap_or(DEFAULT_AUDIT_PAGE_SIZE)
        .clamp(1, MAX_AUDIT_PAGE_SIZE)
}

fn source_rank(source: &MemoryAuditSource) -> usize {
    match source {
        MemoryAuditSource::ClaimDecision => 0,
        MemoryAuditSource::Experience => 1,
        MemoryAuditSource::LegacyMemory => 2,
    }
}

fn item_source_rank(item: &MemoryAuditItem) -> usize {
    source_rank(&item.source)
}

fn compare_audit_items(a: &MemoryAuditItem, b: &MemoryAuditItem) -> std::cmp::Ordering {
    b.created_at
        .cmp(&a.created_at)
        .then_with(|| item_source_rank(a).cmp(&item_source_rank(b)))
        .then_with(|| a.id.cmp(&b.id))
}

fn memory_item(record: MemoryHistoryRecord) -> MemoryAuditItem {
    MemoryAuditItem {
        source: MemoryAuditSource::LegacyMemory,
        id: record.id.clone(),
        created_at: record.created_at.clone(),
        item: MemoryAuditRecord::LegacyMemory(record),
    }
}

fn experience_item(record: MemoryExperienceHistoryRecord) -> MemoryAuditItem {
    MemoryAuditItem {
        source: MemoryAuditSource::Experience,
        id: record.id.clone(),
        created_at: record.created_at.clone(),
        item: MemoryAuditRecord::Experience(record),
    }
}

fn decision_item(record: DreamingDecisionListItem) -> MemoryAuditItem {
    MemoryAuditItem {
        source: MemoryAuditSource::ClaimDecision,
        id: record.id.clone(),
        created_at: record.created_at.clone(),
        item: MemoryAuditRecord::ClaimDecision(record),
    }
}

pub fn merge_memory_audit_items(
    mut items: Vec<MemoryAuditItem>,
    offset: usize,
    limit: usize,
) -> Vec<MemoryAuditItem> {
    items.sort_by(compare_audit_items);
    items.into_iter().skip(offset).take(limit).collect()
}

pub fn build_memory_audit_page_response(
    legacy: MemoryHistoryListResponse,
    experience: Option<MemoryExperienceHistoryListPage>,
    decisions: Option<dreaming::DreamingDecisionListResponse>,
    offset: usize,
    limit: usize,
) -> MemoryAuditPageResponse {
    let include_cross_source = experience.is_some() || decisions.is_some();
    let experience_total = experience.as_ref().map(|page| page.total).unwrap_or(0);
    let experience_truncated = experience
        .as_ref()
        .map(|page| page.total_truncated)
        .unwrap_or(false);
    let decision_total = decisions.as_ref().map(|page| page.total).unwrap_or(0);
    let decision_truncated = decisions
        .as_ref()
        .map(|page| page.total_truncated)
        .unwrap_or(false);

    let mut items: Vec<MemoryAuditItem> = legacy.items.into_iter().map(memory_item).collect();
    if let Some(page) = experience {
        items.extend(page.items.into_iter().map(experience_item));
    }
    if let Some(page) = decisions {
        items.extend(page.items.into_iter().map(decision_item));
    }

    let total = legacy.total + experience_total + decision_total;
    MemoryAuditPageResponse {
        items: merge_memory_audit_items(items, offset, limit),
        total,
        total_truncated: legacy.total_truncated || experience_truncated || decision_truncated,
        sources: MemoryAuditSourceSummaries {
            legacy_memory: MemoryAuditSourceSummary {
                included: true,
                total: legacy.total,
                total_truncated: legacy.total_truncated,
            },
            experience: MemoryAuditSourceSummary {
                included: include_cross_source,
                total: experience_total,
                total_truncated: experience_truncated,
            },
            claim_decision: MemoryAuditSourceSummary {
                included: include_cross_source,
                total: decision_total,
                total_truncated: decision_truncated,
            },
        },
    }
}

pub fn memory_audit_page(query: MemoryAuditPageQuery) -> Result<MemoryAuditPageResponse> {
    let legacy_action = normalized_action(query.action.as_deref())?;
    let include_cross_source = legacy_action.is_none();
    let offset = query.offset.unwrap_or(0);
    let limit = clamped_limit(query.limit);
    let requested_window = offset.saturating_add(limit);
    let window_truncated = requested_window > MAX_AUDIT_WINDOW;
    let window = requested_window.clamp(limit, MAX_AUDIT_WINDOW);
    let normalized_query = query
        .query
        .map(|q| q.trim().to_string())
        .filter(|q| !q.is_empty());

    let backend =
        crate::get_memory_backend().ok_or_else(|| anyhow!("Memory backend not initialized"))?;
    let legacy = backend.history_filtered_page(&MemoryHistoryQuery {
        query: normalized_query.clone(),
        actions: legacy_action.map(|action| vec![action]),
        memory_types: None,
        sources: None,
        limit: Some(window),
        offset: Some(0),
    })?;

    let experience = if include_cross_source {
        Some(list_experience_history_page(
            MemoryExperienceHistoryQuery {
                query: normalized_query.clone(),
                limit: Some(window),
                offset: Some(0),
                ..Default::default()
            },
        )?)
    } else {
        None
    };

    let decisions = if include_cross_source {
        Some(dreaming::list_decisions_page(DreamingDecisionListFilter {
            query: normalized_query,
            target_type: Some("claim".to_string()),
            limit: Some(window),
            offset: Some(0),
            ..Default::default()
        })?)
    } else {
        None
    };

    let mut response =
        build_memory_audit_page_response(legacy, experience, decisions, offset, limit);
    response.total_truncated = response.total_truncated || window_truncated;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{MemoryScope, MemoryType};

    fn memory(id: &str, created_at: &str) -> MemoryHistoryRecord {
        MemoryHistoryRecord {
            id: id.to_string(),
            memory_id: 1,
            action: MemoryHistoryAction::Update,
            memory_type: MemoryType::User,
            scope: MemoryScope::Global,
            source: "user".to_string(),
            source_session_id: None,
            content_preview: id.to_string(),
            pinned: false,
            created_at: created_at.to_string(),
        }
    }

    fn experience(id: &str, created_at: &str) -> MemoryExperienceHistoryRecord {
        MemoryExperienceHistoryRecord {
            id: id.to_string(),
            target_kind: "procedure".to_string(),
            target_id: format!("target-{id}"),
            action: "update".to_string(),
            scope: MemoryScope::Global,
            title_preview: id.to_string(),
            content_preview: id.to_string(),
            created_at: created_at.to_string(),
        }
    }

    fn decision(id: &str, created_at: &str) -> DreamingDecisionListItem {
        DreamingDecisionListItem {
            id: id.to_string(),
            run_id: format!("run-{id}"),
            decision_type: "archive".to_string(),
            target_type: "claim".to_string(),
            target_id: Some(format!("claim-{id}")),
            score: None,
            rationale: id.to_string(),
            before_json: None,
            after_json: None,
            created_at: created_at.to_string(),
            run_trigger: "manual".to_string(),
            run_phase: "review".to_string(),
            run_status: "completed".to_string(),
            content: Some(id.to_string()),
            scope_type: Some("global".to_string()),
            scope_id: None,
        }
    }

    fn item_labels(items: &[MemoryAuditItem]) -> Vec<String> {
        items
            .iter()
            .map(|item| format!("{:?}:{}", item.source, item.id))
            .collect()
    }

    #[test]
    fn merge_orders_cross_source_ties_deterministically() {
        let at = "2026-07-07T10:00:00Z";
        let response = build_memory_audit_page_response(
            MemoryHistoryListResponse {
                items: vec![memory("m2", at), memory("m1", at)],
                total: 2,
                total_truncated: false,
            },
            Some(MemoryExperienceHistoryListPage {
                items: vec![experience("w2", at), experience("w1", at)],
                total: 2,
                total_truncated: false,
            }),
            Some(dreaming::DreamingDecisionListResponse {
                items: vec![decision("d2", at), decision("d1", at)],
                total: 2,
                total_truncated: false,
            }),
            0,
            10,
        );

        assert_eq!(
            item_labels(&response.items),
            vec![
                "ClaimDecision:d1",
                "ClaimDecision:d2",
                "Experience:w1",
                "Experience:w2",
                "LegacyMemory:m1",
                "LegacyMemory:m2",
            ]
        );
        assert_eq!(response.total, 6);
        assert!(response.sources.experience.included);
        assert!(response.sources.claim_decision.included);
    }

    #[test]
    fn build_response_keeps_legacy_action_views_legacy_only() {
        let response = build_memory_audit_page_response(
            MemoryHistoryListResponse {
                items: vec![memory("m1", "2026-07-07T10:00:00Z")],
                total: 1,
                total_truncated: false,
            },
            None,
            None,
            0,
            10,
        );

        assert_eq!(item_labels(&response.items), vec!["LegacyMemory:m1"]);
        assert_eq!(response.total, 1);
        assert!(!response.sources.experience.included);
        assert!(!response.sources.claim_decision.included);
    }

    #[test]
    fn build_response_preserves_source_totals_and_truncation() {
        let response = build_memory_audit_page_response(
            MemoryHistoryListResponse {
                items: vec![memory("m1", "2026-07-07T10:00:00Z")],
                total: 3,
                total_truncated: true,
            },
            Some(MemoryExperienceHistoryListPage {
                items: vec![experience("w1", "2026-07-07T11:00:00Z")],
                total: 4,
                total_truncated: false,
            }),
            Some(dreaming::DreamingDecisionListResponse {
                items: vec![decision("d1", "2026-07-07T12:00:00Z")],
                total: 5,
                total_truncated: true,
            }),
            0,
            10,
        );

        assert_eq!(response.total, 12);
        assert!(response.total_truncated);
        assert_eq!(response.sources.legacy_memory.total, 3);
        assert!(response.sources.legacy_memory.total_truncated);
        assert_eq!(response.sources.experience.total, 4);
        assert!(!response.sources.experience.total_truncated);
        assert_eq!(response.sources.claim_decision.total, 5);
        assert!(response.sources.claim_decision.total_truncated);
    }

    #[test]
    fn audit_action_parser_keeps_all_cross_source_and_rejects_unknown() {
        assert!(normalized_action(None).unwrap().is_none());
        assert!(normalized_action(Some("all")).unwrap().is_none());
        assert!(matches!(
            normalized_action(Some("delete")).unwrap(),
            Some(MemoryHistoryAction::Delete)
        ));
        assert!(normalized_action(Some("restore")).is_err());
    }

    #[test]
    fn merge_applies_global_offset_after_cross_source_sort() {
        let response = build_memory_audit_page_response(
            MemoryHistoryListResponse {
                items: vec![
                    memory("m1", "2026-07-07T10:00:00Z"),
                    memory("m2", "2026-07-07T09:00:00Z"),
                ],
                total: 2,
                total_truncated: false,
            },
            Some(MemoryExperienceHistoryListPage {
                items: vec![experience("w1", "2026-07-07T11:00:00Z")],
                total: 1,
                total_truncated: false,
            }),
            Some(dreaming::DreamingDecisionListResponse {
                items: vec![decision("d1", "2026-07-07T12:00:00Z")],
                total: 1,
                total_truncated: false,
            }),
            1,
            2,
        );

        assert_eq!(
            item_labels(&response.items),
            vec!["Experience:w1", "LegacyMemory:m1"]
        );
        assert_eq!(response.total, 4);
    }
}
