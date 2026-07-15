use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use super::active_memory::{ActiveMemoryRecall, UsedMemoryRef};
use super::related_notes::RelatedNotesRecall;

const DEFAULT_MAX_TRACE_REFS: usize = 24;
const DEFAULT_MAX_CANDIDATES_PER_ORIGIN: usize = 4;
const RANKING_VERSION: &str = "source_fusion_v2";

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalIntent {
    #[default]
    General,
    Profile,
    Procedure,
    Episode,
    Relationship,
    Knowledge,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalPlannerLayerTrace {
    pub layer: String,
    pub status: String,
    #[serde(default)]
    pub ref_count: usize,
    #[serde(default)]
    pub injected_count: usize,
    #[serde(default)]
    pub selected_count: usize,
    #[serde(default)]
    pub candidate_count: usize,
    #[serde(default)]
    pub dropped_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalPlannerTrace {
    pub status: String,
    pub total_refs: usize,
    #[serde(default = "default_ranking_version")]
    pub ranking_version: String,
    #[serde(default)]
    pub intent: RetrievalIntent,
    #[serde(default)]
    pub max_trace_refs: usize,
    #[serde(default)]
    pub max_candidates_per_origin: usize,
    pub layers: Vec<RetrievalPlannerLayerTrace>,
}

fn default_ranking_version() -> String {
    RANKING_VERSION.to_string()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetrievalPlannerRefBudget {
    pub max_total: usize,
    pub max_candidates_per_origin: usize,
}

impl Default for RetrievalPlannerRefBudget {
    fn default() -> Self {
        Self {
            max_total: DEFAULT_MAX_TRACE_REFS,
            max_candidates_per_origin: DEFAULT_MAX_CANDIDATES_PER_ORIGIN,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetrievalPlannerDecisionContext {
    pub budget: RetrievalPlannerRefBudget,
    pub intent: RetrievalIntent,
    pub intent_aware: bool,
}

impl Default for RetrievalPlannerDecisionContext {
    fn default() -> Self {
        Self {
            budget: RetrievalPlannerRefBudget::default(),
            intent: RetrievalIntent::General,
            intent_aware: true,
        }
    }
}

impl RetrievalPlannerDecisionContext {
    pub fn for_query(query: &str, budget: RetrievalPlannerRefBudget, intent_aware: bool) -> Self {
        Self {
            budget,
            intent: if intent_aware {
                classify_intent(query)
            } else {
                RetrievalIntent::General
            },
            intent_aware,
        }
    }
}

pub fn disabled_layer(layer: &str, reason: &str) -> RetrievalPlannerLayerTrace {
    RetrievalPlannerLayerTrace {
        layer: layer.to_string(),
        status: "disabled".to_string(),
        ref_count: 0,
        injected_count: 0,
        selected_count: 0,
        candidate_count: 0,
        dropped_count: 0,
        skipped_reason: Some(reason.to_string()),
        latency_ms: None,
        cached: None,
    }
}

pub fn empty_layer(
    layer: &str,
    reason: &str,
    candidate_count: usize,
) -> RetrievalPlannerLayerTrace {
    RetrievalPlannerLayerTrace {
        layer: layer.to_string(),
        status: "empty".to_string(),
        ref_count: 0,
        injected_count: 0,
        selected_count: 0,
        candidate_count,
        dropped_count: 0,
        skipped_reason: Some(reason.to_string()),
        latency_ms: None,
        cached: None,
    }
}

pub fn skipped_layer(
    layer: &str,
    reason: &str,
    candidate_count: usize,
    latency_ms: Option<u64>,
) -> RetrievalPlannerLayerTrace {
    RetrievalPlannerLayerTrace {
        layer: layer.to_string(),
        status: "skipped".to_string(),
        ref_count: 0,
        injected_count: 0,
        selected_count: 0,
        candidate_count,
        dropped_count: 0,
        skipped_reason: Some(reason.to_string()),
        latency_ms,
        cached: None,
    }
}

pub fn active_layer_from_recall(recall: &ActiveMemoryRecall) -> RetrievalPlannerLayerTrace {
    let refs = recall.used_memory_refs();
    let selected_count = refs.iter().filter(|r| r.role == "selected").count();
    let used_summary = !recall.summary.trim().is_empty();
    RetrievalPlannerLayerTrace {
        layer: "active_memory".to_string(),
        status: if used_summary { "used" } else { "empty" }.to_string(),
        ref_count: refs.len(),
        injected_count: if used_summary {
            selected_count.max(1)
        } else {
            0
        },
        selected_count,
        candidate_count: refs.iter().filter(|r| is_candidate_role(&r.role)).count(),
        dropped_count: 0,
        skipped_reason: (!used_summary).then(|| "llm_none".to_string()),
        latency_ms: recall.latency_ms,
        cached: Some(recall.cached),
    }
}

pub fn knowledge_layer_from_recall(recall: &RelatedNotesRecall) -> RetrievalPlannerLayerTrace {
    RetrievalPlannerLayerTrace {
        layer: "knowledge".to_string(),
        status: "used".to_string(),
        ref_count: recall.refs.len(),
        injected_count: recall.refs.len(),
        selected_count: 0,
        candidate_count: 0,
        dropped_count: 0,
        skipped_reason: None,
        latency_ms: None,
        cached: None,
    }
}

pub fn mark_cached(mut layer: RetrievalPlannerLayerTrace) -> RetrievalPlannerLayerTrace {
    layer.cached = Some(true);
    layer
}

pub fn upsert_layer(
    layers: &mut Vec<RetrievalPlannerLayerTrace>,
    layer: RetrievalPlannerLayerTrace,
) {
    layers.retain(|existing| existing.layer != layer.layer);
    layers.push(layer);
}

#[cfg(test)]
pub fn build_trace(
    refs: &[UsedMemoryRef],
    layers: Vec<RetrievalPlannerLayerTrace>,
) -> Option<RetrievalPlannerTrace> {
    build_trace_with_context(refs, layers, RetrievalPlannerDecisionContext::default())
}

pub fn build_trace_with_context(
    refs: &[UsedMemoryRef],
    mut layers: Vec<RetrievalPlannerLayerTrace>,
    context: RetrievalPlannerDecisionContext,
) -> Option<RetrievalPlannerTrace> {
    add_ref_layer(&mut layers, refs, "context_pack", |origin| {
        origin == "pinned_memory" || origin.starts_with("context_pack:")
    });
    add_ref_layer(&mut layers, refs, "static_memory", |origin| {
        origin == "static_memory"
    });
    add_ref_layer(&mut layers, refs, "profile", |origin| origin == "profile");
    add_ref_layer(&mut layers, refs, "knowledge", |origin| {
        origin == "knowledge"
    });
    add_ref_layer(&mut layers, refs, "active_memory", |origin| {
        origin == "active_memory"
    });
    add_ref_layer(&mut layers, refs, "experience", |origin| {
        origin == "experience"
    });
    add_ref_layer(&mut layers, refs, "graph", |origin| origin == "graph");
    reconcile_layer_counts(&mut layers, refs);

    if refs.is_empty() && layers.is_empty() {
        return None;
    }

    layers.sort_by_key(|layer| layer_order(&layer.layer));
    Some(RetrievalPlannerTrace {
        status: summarize_status(refs, &layers).to_string(),
        total_refs: refs.len(),
        ranking_version: RANKING_VERSION.to_string(),
        intent: context.intent,
        max_trace_refs: context.budget.max_total,
        max_candidates_per_origin: context.budget.max_candidates_per_origin,
        layers,
    })
}

#[cfg(test)]
pub fn select_refs_for_trace_with_budget(
    refs: Vec<UsedMemoryRef>,
    budget: RetrievalPlannerRefBudget,
) -> Vec<UsedMemoryRef> {
    select_refs_for_trace_with_context(
        refs,
        RetrievalPlannerDecisionContext {
            budget,
            intent: RetrievalIntent::General,
            intent_aware: false,
        },
    )
}

pub fn select_refs_for_trace_with_context(
    refs: Vec<UsedMemoryRef>,
    context: RetrievalPlannerDecisionContext,
) -> Vec<UsedMemoryRef> {
    if refs.is_empty() {
        return refs;
    }
    if context.budget.max_total == 0 {
        return Vec::new();
    }
    let mut seen_primary = HashSet::new();
    let mut primary = Vec::new();
    let mut raw_candidates = Vec::new();

    for (index, reference) in refs.into_iter().enumerate() {
        if is_candidate_role(&reference.role) {
            raw_candidates.push((index, reference));
        } else if seen_primary.insert(ref_identity_key(&reference)) {
            primary.push(reference);
        }
    }

    if primary.len() >= context.budget.max_total {
        return primary;
    }

    let primary_items = primary
        .iter()
        .map(canonical_item_key)
        .collect::<HashSet<_>>();
    let mut source_positions = HashMap::<String, usize>::new();
    let mut candidates_by_item = HashMap::<String, RankedCandidate>::new();
    for (input_index, reference) in raw_candidates {
        let item_key = canonical_item_key(&reference);
        if primary_items.contains(&item_key) {
            continue;
        }
        let source_position = source_positions
            .entry(reference.origin.clone())
            .or_default();
        let rank = candidate_rank(&reference, *source_position, context);
        *source_position += 1;
        let candidate = RankedCandidate {
            input_index,
            rank,
            reference,
        };
        match candidates_by_item.entry(item_key) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(candidate);
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                if ranked_candidate_cmp(&candidate, entry.get()).is_lt() {
                    entry.insert(candidate);
                }
            }
        }
    }

    let mut candidates = candidates_by_item.into_values().collect::<Vec<_>>();
    candidates.sort_by(ranked_candidate_cmp);

    let mut per_origin = HashMap::<String, usize>::new();
    let mut selected = primary;
    for candidate in candidates {
        if selected.len() >= context.budget.max_total {
            break;
        }
        let reference = candidate.reference;
        let count = per_origin.entry(reference.origin.clone()).or_default();
        if *count >= context.budget.max_candidates_per_origin {
            continue;
        }
        *count += 1;
        selected.push(reference);
    }
    selected
}

#[derive(Debug)]
struct RankedCandidate {
    input_index: usize,
    rank: i64,
    reference: UsedMemoryRef,
}

fn ranked_candidate_cmp(left: &RankedCandidate, right: &RankedCandidate) -> std::cmp::Ordering {
    right
        .rank
        .cmp(&left.rank)
        .then_with(|| left.reference.origin.cmp(&right.reference.origin))
        .then_with(|| left.reference.kind.cmp(&right.reference.kind))
        .then_with(|| left.reference.id.cmp(&right.reference.id))
        .then_with(|| left.input_index.cmp(&right.input_index))
}

fn ref_identity_key(reference: &UsedMemoryRef) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}",
        reference.origin,
        normalized_role_for_identity(&reference.role),
        reference.kind,
        reference.id
    )
}

fn canonical_item_key(reference: &UsedMemoryRef) -> String {
    format!("{}\u{1f}{}", reference.kind, reference.id)
}

fn is_candidate_role(role: &str) -> bool {
    role == "candidate" || role == "considered"
}

fn normalized_role_for_identity(role: &str) -> &str {
    if is_candidate_role(role) {
        "candidate"
    } else {
        role
    }
}

fn candidate_rank(
    reference: &UsedMemoryRef,
    source_position: usize,
    context: RetrievalPlannerDecisionContext,
) -> i64 {
    scope_rank(&reference.scope)
        + source_prior(reference)
        + source_position_rank(source_position)
        + intent_rank(reference, context)
        + scaled_metric(reference.salience, 100)
        + scaled_metric(reference.confidence, 120)
        + scaled_metric(reference.score, 140)
}

fn scope_rank(scope: &str) -> i64 {
    let normalized = scope.trim().to_ascii_lowercase();
    if normalized.starts_with("project:") {
        500
    } else if normalized.starts_with("agent:") {
        250
    } else {
        0
    }
}

fn source_prior(reference: &UsedMemoryRef) -> i64 {
    match reference.origin.as_str() {
        "active_memory" => 100,
        "knowledge" => 80,
        "experience" => 70,
        "graph" => 60,
        "static_memory" | "pinned_memory" => 100,
        "profile" => 90,
        _ => 40,
    }
}

fn source_position_rank(position: usize) -> i64 {
    40 / (position.saturating_add(1) as i64)
}

fn intent_rank(reference: &UsedMemoryRef, context: RetrievalPlannerDecisionContext) -> i64 {
    if !context.intent_aware {
        return 0;
    }
    match context.intent {
        RetrievalIntent::General => 0,
        RetrievalIntent::Profile => match reference.origin.as_str() {
            "active_memory" => 420,
            "profile" | "static_memory" | "pinned_memory" => 320,
            "graph" => 100,
            _ => 0,
        },
        RetrievalIntent::Procedure => {
            if reference.origin == "experience" && reference.kind == "procedure" {
                560
            } else if reference.origin == "experience" {
                180
            } else if reference.origin == "knowledge" {
                100
            } else {
                0
            }
        }
        RetrievalIntent::Episode => {
            if reference.origin == "experience" && reference.kind == "episode" {
                560
            } else if reference.origin == "experience" {
                220
            } else if reference.origin == "active_memory" {
                100
            } else {
                0
            }
        }
        RetrievalIntent::Relationship => match reference.origin.as_str() {
            "graph" => 560,
            "active_memory" => 120,
            _ => 0,
        },
        RetrievalIntent::Knowledge => match reference.origin.as_str() {
            "knowledge" => 560,
            "experience" => 120,
            _ => 0,
        },
    }
}

pub fn classify_intent(query: &str) -> RetrievalIntent {
    let normalized = query.trim().to_lowercase();
    if normalized.is_empty() {
        return RetrievalIntent::General;
    }
    // Temporal markers win when a query also contains a procedural phrase.
    // “之前如何处理冲突” asks for what happened in the prior episode, not a
    // generic how-to. Keeping this overlap deterministic prevents procedure
    // memories from displacing the user's actual historical context.
    if contains_any(
        &normalized,
        &[
            "last time",
            "previously",
            "what happened",
            "when did",
            "上次",
            "之前",
            "曾经",
            "曾經",
            "经历",
            "經歷",
            "发生过",
            "發生過",
            "前回",
            "以前",
            "何が起きた",
            "いつだった",
            "지난번",
            "이전에",
            "무슨 일이",
            "언제",
            "última vez",
            "anteriormente",
            "qué pasó",
            "cuándo",
            "kali terakhir",
            "sebelum ini",
            "apa yang berlaku",
            "bila",
            "o que aconteceu",
            "quando",
            "в прошлый раз",
            "ранее",
            "что произошло",
            "когда",
            "geçen sefer",
            "daha önce",
            "ne oldu",
            "ne zaman",
            "lần trước",
            "trước đây",
            "chuyện gì đã xảy ra",
            "khi nào",
            "المرة الماضية",
            "سابقًا",
            "ماذا حدث",
            "متى",
        ],
    ) {
        RetrievalIntent::Episode
    } else if contains_any(
        &normalized,
        &[
            "how do",
            "how to",
            "steps",
            "workflow",
            "procedure",
            "process",
            "recipe",
            "怎么做",
            "如何",
            "步骤",
            "步驟",
            "流程",
            "做法",
            "どうやって",
            "手順",
            "方法",
            "ワークフロー",
            "プロセス",
            "어떻게",
            "단계",
            "절차",
            "워크플로",
            "과정",
            "cómo",
            "pasos",
            "flujo de trabajo",
            "procedimiento",
            "proceso",
            "receta",
            "bagaimana",
            "langkah",
            "aliran kerja",
            "prosedur",
            "proses",
            "etapas",
            "fluxo de trabalho",
            "procedimento",
            "как",
            "шаги",
            "рабочий процесс",
            "процедура",
            "процесс",
            "nasıl",
            "adımlar",
            "iş akışı",
            "prosedür",
            "süreç",
            "làm thế nào",
            "các bước",
            "quy trình",
            "thủ tục",
            "كيف",
            "خطوات",
            "سير العمل",
            "إجراء",
            "عملية",
        ],
    ) {
        RetrievalIntent::Procedure
    } else if contains_any(
        &normalized,
        &[
            "note",
            "notes",
            "document",
            "docs",
            "knowledge",
            "file",
            "笔记",
            "筆記",
            "文档",
            "文檔",
            "文件",
            "资料",
            "資料",
            "知识库",
            "知識庫",
            "ノート",
            "メモ",
            "文書",
            "ドキュメント",
            "ナレッジ",
            "ファイル",
            "노트",
            "문서",
            "자료",
            "지식",
            "파일",
            "nota",
            "notas",
            "documento",
            "documentación",
            "conocimiento",
            "archivo",
            "dokumen",
            "pengetahuan",
            "fail dokumen",
            "documentação",
            "conhecimento",
            "arquivo",
            "заметка",
            "заметки",
            "документ",
            "документация",
            "знание",
            "файл",
            "notlar",
            "not defteri",
            "belge",
            "doküman",
            "bilgi",
            "dosya",
            "ghi chú",
            "tài liệu",
            "kiến thức",
            "tệp",
            "ملاحظة",
            "ملاحظات",
            "مستند",
            "وثيقة",
            "معرفة",
            "ملف",
        ],
    ) {
        RetrievalIntent::Knowledge
    } else if contains_any(
        &normalized,
        &[
            "related",
            "relationship",
            "depends on",
            "connected",
            "关联",
            "關聯",
            "关系",
            "關係",
            "依赖",
            "依賴",
            "相关",
            "相關",
            "接続",
            "연관",
            "관련",
            "관계",
            "의존",
            "연결",
            "relacionado",
            "relación",
            "depende de",
            "conectado",
            "berkaitan",
            "hubungan",
            "bergantung pada",
            "tersambung",
            "relação",
            "связан",
            "отношение",
            "зависит от",
            "соединён",
            "ilgili",
            "ilişki",
            "bağlı",
            "bağlantılı",
            "liên quan",
            "mối quan hệ",
            "phụ thuộc",
            "kết nối",
            "مرتبط",
            "علاقة",
            "يعتمد على",
            "متصل",
        ],
    ) {
        RetrievalIntent::Relationship
    } else if contains_any(
        &normalized,
        &[
            "my preference",
            "my profile",
            "i prefer",
            "i like",
            "about me",
            "remember me",
            "my usual",
            "as usual",
            "我的",
            "偏好",
            "喜欢",
            "喜歡",
            "我叫",
            "平时",
            "平時",
            "习惯",
            "習慣",
            "私の好み",
            "好き",
            "私について",
            "覚えて",
            "いつもの",
            "普段",
            "習慣",
            "내 취향",
            "선호",
            "좋아",
            "나에 대해",
            "기억해",
            "평소",
            "습관",
            "mi preferencia",
            "prefiero",
            "me gusta",
            "sobre mí",
            "recuérdame",
            "suelo",
            "como siempre",
            "pilihan saya",
            "saya lebih suka",
            "saya suka",
            "tentang saya",
            "ingat saya",
            "kebiasaan saya",
            "seperti biasa",
            "minha preferência",
            "eu prefiro",
            "eu gosto",
            "sobre mim",
            "lembre de mim",
            "costumo",
            "meus hábitos",
            "мои предпочтения",
            "я предпочитаю",
            "мне нравится",
            "обо мне",
            "запомни меня",
            "обычно",
            "привычка",
            "tercihim",
            "tercih ederim",
            "severim",
            "hakkımda",
            "beni hatırla",
            "genelde",
            "alışkanlığım",
            "sở thích của tôi",
            "tôi thích",
            "về tôi",
            "nhớ tôi",
            "thường lệ",
            "thói quen",
            "تفضيلاتي",
            "أفضل",
            "أحب",
            "عني",
            "تذكرني",
            "عادتي",
            "كالعادة",
        ],
    ) {
        RetrievalIntent::Profile
    } else {
        RetrievalIntent::General
    }
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| contains_marker(value, needle))
}

fn contains_marker(value: &str, marker: &str) -> bool {
    // CJK, Kana and Hangul are commonly written without spaces, so substring
    // matching is intentional for those scripts. Space-delimited scripts use
    // Unicode alphanumeric boundaries to avoid collisions such as the Turkish
    // word "not" matching English "nothing", or "file" matching "profile".
    if marker.chars().any(is_compact_script_char) {
        return value.contains(marker);
    }
    value.match_indices(marker).any(|(start, matched)| {
        let before_ok = value[..start]
            .chars()
            .next_back()
            .is_none_or(|ch| !ch.is_alphanumeric());
        let end = start + matched.len();
        let after_ok = value[end..]
            .chars()
            .next()
            .is_none_or(|ch| !ch.is_alphanumeric());
        before_ok && after_ok
    })
}

fn is_compact_script_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{3040}'..='\u{30ff}' | '\u{3400}'..='\u{9fff}' | '\u{ac00}'..='\u{d7af}'
    )
}

fn scaled_metric(value: Option<f32>, weight: i64) -> i64 {
    let Some(value) = value else {
        return 0;
    };
    if !value.is_finite() {
        return 0;
    }
    (value.clamp(0.0, 1.0) * weight as f32).round() as i64
}

fn summarize_status(refs: &[UsedMemoryRef], layers: &[RetrievalPlannerLayerTrace]) -> &'static str {
    let has_refs = !refs.is_empty();
    let has_used_context = refs
        .iter()
        .any(|reference| !is_candidate_role(&reference.role))
        || layers.iter().any(|layer| layer.status == "used");
    let has_degraded_layer = layers.iter().any(is_degraded_layer);
    if has_used_context {
        return if has_degraded_layer {
            "partial"
        } else {
            "used"
        };
    }
    if has_degraded_layer {
        return "degraded";
    }
    if has_refs || layers.iter().any(|layer| layer.status == "candidate") {
        return "candidates";
    }
    if !layers.is_empty() && layers.iter().all(|layer| layer.status == "disabled") {
        return "disabled";
    }
    "no_context"
}

fn is_degraded_layer(layer: &RetrievalPlannerLayerTrace) -> bool {
    if layer.status != "skipped" {
        return false;
    }
    !matches!(
        layer.skipped_reason.as_deref(),
        Some("unified_dynamic_recall")
    )
}

fn add_ref_layer<F>(
    layers: &mut Vec<RetrievalPlannerLayerTrace>,
    refs: &[UsedMemoryRef],
    layer: &str,
    origin_matches: F,
) where
    F: Fn(&str) -> bool,
{
    if layers.iter().any(|existing| existing.layer == layer) {
        return;
    }
    let matching: Vec<&UsedMemoryRef> = refs
        .iter()
        .filter(|r| origin_matches(r.origin.as_str()))
        .collect();
    if matching.is_empty() {
        return;
    }
    let injected_count = matching.iter().filter(|r| r.role == "injected").count();
    let selected_count = matching.iter().filter(|r| r.role == "selected").count();
    let candidate_count = matching
        .iter()
        .filter(|r| is_candidate_role(&r.role))
        .count();
    layers.push(RetrievalPlannerLayerTrace {
        layer: layer.to_string(),
        status: if injected_count > 0 || selected_count > 0 {
            "used".to_string()
        } else {
            "candidate".to_string()
        },
        ref_count: matching.len(),
        injected_count,
        selected_count,
        candidate_count,
        dropped_count: 0,
        skipped_reason: None,
        latency_ms: None,
        cached: None,
    });
}

fn reconcile_layer_counts(layers: &mut [RetrievalPlannerLayerTrace], refs: &[UsedMemoryRef]) {
    for layer in layers {
        if !matches!(layer.status.as_str(), "used" | "candidate") {
            continue;
        }
        let matching: Vec<&UsedMemoryRef> = refs
            .iter()
            .filter(|reference| ref_matches_layer(reference, &layer.layer))
            .collect();
        let selected_ref_count = matching.len();
        let selected_injected = matching.iter().filter(|r| r.role == "injected").count();
        let selected_selected = matching.iter().filter(|r| r.role == "selected").count();
        let selected_candidates = matching
            .iter()
            .filter(|r| is_candidate_role(&r.role))
            .count();
        let previous_ref_count = layer.ref_count;
        layer.ref_count = selected_ref_count;
        layer.injected_count = selected_injected;
        layer.selected_count = selected_selected;
        layer.candidate_count = selected_candidates;
        layer.dropped_count = previous_ref_count.saturating_sub(selected_ref_count);
    }
}

fn ref_matches_layer(reference: &UsedMemoryRef, layer: &str) -> bool {
    match layer {
        "context_pack" => {
            reference.origin == "pinned_memory" || reference.origin.starts_with("context_pack:")
        }
        "static_memory" => reference.origin == "static_memory",
        "profile" => reference.origin == "profile",
        "knowledge" => reference.origin == "knowledge",
        "active_memory" => reference.origin == "active_memory",
        "experience" => reference.origin == "experience",
        "graph" => reference.origin == "graph",
        other => reference.origin == other,
    }
}

fn layer_order(layer: &str) -> usize {
    match layer {
        "context_pack" => 0,
        "static_memory" => 1,
        "profile" => 2,
        "active_memory" => 3,
        "graph" => 4,
        "experience" => 5,
        "knowledge" => 6,
        _ => 99,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn used_ref(origin: &str, role: &str) -> UsedMemoryRef {
        UsedMemoryRef {
            kind: "memory".to_string(),
            id: format!("{origin}:{role}"),
            source_type: "user".to_string(),
            scope: "global".to_string(),
            origin: origin.to_string(),
            role: role.to_string(),
            preview: "preview".to_string(),
            path: None,
            line: None,
            col: None,
            heading_path: None,
            block_id: None,
            score: None,
            confidence: None,
            salience: None,
        }
    }

    fn candidate_ref(origin: &str, id: &str, salience: f32) -> UsedMemoryRef {
        UsedMemoryRef {
            kind: "claim".to_string(),
            id: id.to_string(),
            source_type: "preference".to_string(),
            scope: "global".to_string(),
            origin: origin.to_string(),
            role: "candidate".to_string(),
            preview: format!("{origin}:{id}"),
            path: None,
            line: None,
            col: None,
            heading_path: None,
            block_id: None,
            score: Some(0.2),
            confidence: Some(0.5),
            salience: Some(salience),
        }
    }

    fn scoped_candidate_ref(origin: &str, id: &str, scope: &str, salience: f32) -> UsedMemoryRef {
        UsedMemoryRef {
            scope: scope.to_string(),
            ..candidate_ref(origin, id, salience)
        }
    }

    fn scoped_considered_ref(origin: &str, id: &str, scope: &str, salience: f32) -> UsedMemoryRef {
        UsedMemoryRef {
            role: "considered".to_string(),
            ..scoped_candidate_ref(origin, id, scope, salience)
        }
    }

    fn typed_candidate_ref(origin: &str, kind: &str, id: &str, scope: &str) -> UsedMemoryRef {
        UsedMemoryRef {
            kind: kind.to_string(),
            ..scoped_candidate_ref(origin, id, scope, 0.5)
        }
    }

    #[test]
    fn build_trace_summarizes_ref_layers_without_changing_refs() {
        let refs = vec![
            used_ref("static_memory", "injected"),
            used_ref("active_memory", "selected"),
            used_ref("active_memory", "candidate"),
            used_ref("graph", "candidate"),
            used_ref("experience", "candidate"),
        ];

        let trace = build_trace(&refs, Vec::new()).unwrap();

        assert_eq!(trace.status, "used");
        assert_eq!(trace.total_refs, 5);
        assert_eq!(trace.ranking_version, RANKING_VERSION);
        assert_eq!(trace.intent, RetrievalIntent::General);
        assert_eq!(trace.max_trace_refs, DEFAULT_MAX_TRACE_REFS);
        assert!(trace
            .layers
            .iter()
            .any(|l| l.layer == "static_memory" && l.injected_count == 1));
        assert!(trace.layers.iter().any(|l| l.layer == "active_memory"
            && l.selected_count == 1
            && l.candidate_count == 1));
        assert!(trace
            .layers
            .iter()
            .any(|l| l.layer == "graph" && l.status == "candidate" && l.candidate_count == 1));
        assert!(trace
            .layers
            .iter()
            .any(|l| l.layer == "experience" && l.status == "candidate" && l.candidate_count == 1));
    }

    #[test]
    fn build_trace_keeps_explicit_empty_layer_when_no_refs() {
        let trace =
            build_trace(&[], vec![empty_layer("active_memory", "no_candidates", 0)]).unwrap();

        assert_eq!(trace.status, "no_context");
        assert_eq!(trace.total_refs, 0);
        assert_eq!(trace.layers[0].status, "empty");
        assert_eq!(
            trace.layers[0].skipped_reason.as_deref(),
            Some("no_candidates")
        );
    }

    #[test]
    fn build_trace_preserves_skipped_retrieval_error_layer_with_latency() {
        let trace = build_trace(
            &[],
            vec![skipped_layer("graph", "retrieval_error", 0, Some(37))],
        )
        .unwrap();

        assert_eq!(trace.status, "degraded");
        assert_eq!(trace.total_refs, 0);
        assert_eq!(trace.layers.len(), 1);
        assert_eq!(trace.layers[0].layer, "graph");
        assert_eq!(trace.layers[0].status, "skipped");
        assert_eq!(
            trace.layers[0].skipped_reason.as_deref(),
            Some("retrieval_error")
        );
        assert_eq!(trace.layers[0].latency_ms, Some(37));
    }

    #[test]
    fn build_trace_reports_partial_when_some_layers_used_and_some_degraded() {
        let trace = build_trace(
            &[used_ref("static_memory", "injected")],
            vec![skipped_layer("graph", "retrieval_error", 0, Some(37))],
        )
        .unwrap();

        assert_eq!(trace.status, "partial");
        assert_eq!(trace.total_refs, 1);
        assert!(trace
            .layers
            .iter()
            .any(|layer| layer.layer == "static_memory" && layer.status == "used"));
        assert!(trace
            .layers
            .iter()
            .any(|layer| layer.layer == "graph" && layer.status == "skipped"));
    }

    #[test]
    fn build_trace_reports_disabled_when_all_layers_are_disabled() {
        let trace = build_trace(
            &[],
            vec![
                disabled_layer("active_memory", "disabled"),
                disabled_layer("knowledge", "no_access"),
            ],
        )
        .unwrap();

        assert_eq!(trace.status, "disabled");
        assert_eq!(trace.total_refs, 0);
        assert_eq!(trace.layers.len(), 2);
    }

    #[test]
    fn build_trace_reports_candidates_when_refs_are_only_considered() {
        let refs = vec![
            used_ref("graph", "candidate"),
            used_ref("experience", "considered"),
        ];

        let trace = build_trace(&refs, Vec::new()).unwrap();

        assert_eq!(trace.status, "candidates");
        assert_eq!(trace.total_refs, 2);
        assert!(trace.layers.iter().all(|layer| layer.status == "candidate"));
        assert!(trace.layers.iter().all(|layer| layer.candidate_count == 1));
    }

    #[test]
    fn select_refs_keeps_injected_and_selected_before_candidate_budget() {
        let refs = vec![
            used_ref("static_memory", "injected"),
            used_ref("active_memory", "selected"),
            candidate_ref("graph", "low", 0.1),
            candidate_ref("graph", "high", 0.9),
            candidate_ref("experience", "mid", 0.5),
        ];

        let selected = select_refs_for_trace_with_budget(
            refs,
            RetrievalPlannerRefBudget {
                max_total: 3,
                max_candidates_per_origin: 2,
            },
        );

        assert_eq!(selected.len(), 3);
        assert_eq!(selected[0].role, "injected");
        assert_eq!(selected[1].role, "selected");
        assert_eq!(selected[2].id, "high");
    }

    #[test]
    fn select_refs_limits_candidate_noise_per_origin() {
        let refs = vec![
            candidate_ref("graph", "g1", 0.9),
            candidate_ref("graph", "g2", 0.8),
            candidate_ref("graph", "g3", 0.7),
            candidate_ref("experience", "e1", 0.6),
            candidate_ref("experience", "e2", 0.5),
        ];

        let selected = select_refs_for_trace_with_budget(
            refs,
            RetrievalPlannerRefBudget {
                max_total: 5,
                max_candidates_per_origin: 2,
            },
        );

        assert_eq!(selected.len(), 4);
        assert_eq!(
            selected
                .iter()
                .filter(|reference| reference.origin == "graph")
                .count(),
            2
        );
        assert!(!selected.iter().any(|reference| reference.id == "g3"));
    }

    #[test]
    fn select_refs_prefers_nearer_scope_when_budgeting_cross_source_candidates() {
        let refs = vec![
            scoped_candidate_ref("graph", "global-graph", "global", 0.9),
            scoped_candidate_ref("experience", "project-workflow", "project:p1", 0.4),
            scoped_candidate_ref("active_memory", "agent-pref", "agent:a1", 0.5),
        ];

        let selected = select_refs_for_trace_with_budget(
            refs,
            RetrievalPlannerRefBudget {
                max_total: 2,
                max_candidates_per_origin: 2,
            },
        );

        assert_eq!(
            selected
                .iter()
                .map(|reference| reference.id.as_str())
                .collect::<Vec<_>>(),
            vec!["project-workflow", "agent-pref"]
        );
    }

    #[test]
    fn select_refs_ranks_considered_role_as_candidate_alias() {
        let refs = vec![
            candidate_ref("graph", "global-graph", 0.1),
            scoped_considered_ref("experience", "project-workflow", "project:p1", 0.9),
        ];

        let selected = select_refs_for_trace_with_budget(
            refs,
            RetrievalPlannerRefBudget {
                max_total: 1,
                max_candidates_per_origin: 2,
            },
        );

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].id, "project-workflow");
        assert_eq!(selected[0].role, "considered");
    }

    #[test]
    fn intent_classifier_covers_english_and_chinese_queries() {
        assert_eq!(
            classify_intent("How do I deploy this release?"),
            RetrievalIntent::Procedure
        );
        assert_eq!(
            classify_intent("上次发布发生过什么？"),
            RetrievalIntent::Episode
        );
        assert_eq!(
            classify_intent("之前如何处理冲突？"),
            RetrievalIntent::Episode
        );
        assert_eq!(
            classify_intent("打开相关设计文档"),
            RetrievalIntent::Knowledge
        );
        assert_eq!(
            classify_intent("这个模块依赖关系是什么"),
            RetrievalIntent::Relationship
        );
        assert_eq!(
            classify_intent("我的回答偏好是什么"),
            RetrievalIntent::Profile
        );
        assert_eq!(classify_intent("summarize this"), RetrievalIntent::General);
    }

    #[test]
    fn intent_classifier_covers_every_supported_ui_language() {
        let cases = [
            ("前回のリリースで何が起きた？", RetrievalIntent::Episode),
            ("이 작업은 어떻게 배포해?", RetrievalIntent::Procedure),
            ("abre el documento de diseño", RetrievalIntent::Knowledge),
            (
                "qual é a relação entre estes módulos?",
                RetrievalIntent::Relationship,
            ),
            ("учти мои предпочтения", RetrievalIntent::Profile),
            ("geçen sefer ne oldu?", RetrievalIntent::Episode),
            ("các bước triển khai là gì?", RetrievalIntent::Procedure),
            ("buka dokumen reka bentuk", RetrievalIntent::Knowledge),
            ("ما تفضيلاتي في الإجابة؟", RetrievalIntent::Profile),
            ("依照我的習慣回答", RetrievalIntent::Profile),
        ];
        for (query, expected) in cases {
            assert_eq!(classify_intent(query), expected, "query={query}");
        }
    }

    #[test]
    fn intent_markers_do_not_match_inside_unrelated_words() {
        assert_eq!(classify_intent("show my profile"), RetrievalIntent::Profile);
        assert_eq!(classify_intent("nothing special"), RetrievalIntent::General);
        assert_eq!(
            classify_intent("this is not relevant"),
            RetrievalIntent::General
        );
        assert_eq!(
            classify_intent("the build may fail"),
            RetrievalIntent::General
        );
    }

    #[test]
    fn query_intent_fuses_sources_without_overriding_injected_context() {
        let refs = vec![
            used_ref("static_memory", "injected"),
            typed_candidate_ref("active_memory", "memory", "fact", "agent:a1"),
            typed_candidate_ref("experience", "procedure", "workflow", "global"),
            typed_candidate_ref("graph", "claim", "edge", "project:p1"),
        ];
        let context = RetrievalPlannerDecisionContext::for_query(
            "How do I run the release workflow?",
            RetrievalPlannerRefBudget {
                max_total: 3,
                max_candidates_per_origin: 2,
            },
            true,
        );

        let selected = select_refs_for_trace_with_context(refs, context);

        assert_eq!(selected[0].role, "injected");
        assert_eq!(selected[1].id, "workflow");
        assert_eq!(selected[2].id, "edge");
    }

    #[test]
    fn cross_source_candidates_deduplicate_by_underlying_item() {
        let refs = vec![
            typed_candidate_ref("graph", "claim", "same-claim", "global"),
            typed_candidate_ref("active_memory", "claim", "same-claim", "project:p1"),
            typed_candidate_ref("experience", "episode", "other", "global"),
        ];

        let selected = select_refs_for_trace_with_budget(
            refs,
            RetrievalPlannerRefBudget {
                max_total: 8,
                max_candidates_per_origin: 4,
            },
        );

        assert_eq!(
            selected
                .iter()
                .filter(|reference| reference.id == "same-claim")
                .count(),
            1
        );
        assert!(selected.iter().any(|reference| {
            reference.id == "same-claim" && reference.origin == "active_memory"
        }));
    }

    #[test]
    fn candidate_ties_use_stable_identity_not_input_order() {
        let forward = vec![
            candidate_ref("source_z", "b", 0.5),
            candidate_ref("source_y", "a", 0.5),
        ];
        let reverse = forward.iter().cloned().rev().collect::<Vec<_>>();
        let budget = RetrievalPlannerRefBudget {
            max_total: 2,
            max_candidates_per_origin: 2,
        };

        let forward_ids = select_refs_for_trace_with_budget(forward, budget)
            .into_iter()
            .map(|reference| reference.id)
            .collect::<Vec<_>>();
        let reverse_ids = select_refs_for_trace_with_budget(reverse, budget)
            .into_iter()
            .map(|reference| reference.id)
            .collect::<Vec<_>>();

        assert_eq!(forward_ids, reverse_ids);
        assert_eq!(forward_ids, vec!["a", "b"]);
    }

    #[test]
    fn build_trace_reconciles_layer_counts_after_ref_budgeting() {
        let refs = vec![
            candidate_ref("graph", "g1", 0.9),
            candidate_ref("graph", "g2", 0.8),
            candidate_ref("graph", "g3", 0.7),
        ];
        let selected = select_refs_for_trace_with_budget(
            refs,
            RetrievalPlannerRefBudget {
                max_total: 2,
                max_candidates_per_origin: 2,
            },
        );

        let trace = build_trace(
            &selected,
            vec![RetrievalPlannerLayerTrace {
                layer: "graph".to_string(),
                status: "candidate".to_string(),
                ref_count: 3,
                injected_count: 0,
                selected_count: 0,
                candidate_count: 3,
                dropped_count: 0,
                skipped_reason: None,
                latency_ms: Some(5),
                cached: None,
            }],
        )
        .unwrap();

        let layer = trace
            .layers
            .iter()
            .find(|layer| layer.layer == "graph")
            .unwrap();
        assert_eq!(trace.total_refs, 2);
        assert_eq!(layer.ref_count, 2);
        assert_eq!(layer.candidate_count, 2);
        assert_eq!(layer.dropped_count, 1);
        assert_eq!(layer.latency_ms, Some(5));
    }

    #[test]
    fn active_layer_legacy_summary_counts_as_used_without_selected_ref() {
        let recall = ActiveMemoryRecall {
            summary: "Use the user's concise-answer preference.".to_string(),
            mode: "legacy".to_string(),
            selected: None,
            selected_candidates: Vec::new(),
            candidates: vec![super::super::active_memory::ActiveMemoryCandidateRef {
                kind: "memory".to_string(),
                id: "1".to_string(),
                source_type: "user".to_string(),
                scope: "global".to_string(),
                preview: "Prefers concise answers.".to_string(),
                score: None,
                confidence: None,
                salience: None,
            }],
            total_candidates: 1,
            latency_ms: Some(12),
            cached: false,
        };

        let layer = active_layer_from_recall(&recall);

        assert_eq!(layer.status, "used");
        assert_eq!(layer.selected_count, 0);
        assert_eq!(layer.candidate_count, 1);
        assert_eq!(layer.skipped_reason, None);
    }

    #[test]
    #[ignore = "opt-in scale benchmark; run pnpm memory:benchmark"]
    fn benchmark_source_fusion_with_one_hundred_thousand_candidates() {
        let candidate_count = std::env::var("HA_MEMORY_BENCH_CANDIDATES")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(100_000)
            .clamp(10_000, 1_000_000);
        let origins = ["active_memory", "graph", "experience", "knowledge"];
        let mut refs = Vec::with_capacity(candidate_count + 2);
        refs.push(used_ref("static_memory", "injected"));
        refs.push(used_ref("active_memory", "selected"));
        for index in 0..candidate_count {
            let origin = origins[index % origins.len()];
            let unique_span = candidate_count.saturating_mul(4).saturating_div(5).max(1);
            let id = format!("item-{:08}", index % unique_span);
            let kind = if origin == "experience" && index % 2 == 0 {
                "procedure"
            } else {
                "claim"
            };
            refs.push(UsedMemoryRef {
                kind: kind.to_string(),
                id,
                source_type: "benchmark".to_string(),
                scope: match index % 5 {
                    0 => "project:p1".to_string(),
                    1 | 2 => "agent:a1".to_string(),
                    _ => "global".to_string(),
                },
                origin: origin.to_string(),
                role: "candidate".to_string(),
                preview: "bounded benchmark preview".to_string(),
                path: None,
                line: None,
                col: None,
                heading_path: None,
                block_id: None,
                score: Some((index % 100) as f32 / 100.0),
                confidence: Some((index % 97) as f32 / 97.0),
                salience: Some((index % 89) as f32 / 89.0),
            });
        }

        let context = RetrievalPlannerDecisionContext::for_query(
            "How do I run the release workflow?",
            RetrievalPlannerRefBudget::default(),
            true,
        );
        let started = std::time::Instant::now();
        let selected = select_refs_for_trace_with_context(refs, context);
        let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
        let identities = selected
            .iter()
            .map(canonical_item_key)
            .collect::<HashSet<_>>();

        println!(
            "RETRIEVAL_PLANNER_SCALE_BENCH {{\"candidates\":{candidate_count},\"selected\":{},\"elapsedMs\":{elapsed_ms:.3},\"intent\":\"procedure\"}}",
            selected.len()
        );
        assert!(selected.len() <= DEFAULT_MAX_TRACE_REFS);
        assert_eq!(identities.len(), selected.len());
        assert_eq!(selected[0].role, "injected");
        assert_eq!(selected[1].role, "selected");
    }
}
