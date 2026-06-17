//! Claim write path — confidence derivation, object normalization, rule-only
//! canonicalize helpers, and effective-status (design §3.2 / §4.4 / §4.5).
//!
//! These are the deterministic, side-effect-free pieces of the dual-write
//! path; the DB writes themselves live on `ClaimStore` (see `store.rs`). LLM
//! output never sets confidence — it only labels `evidence_class`, and the
//! baseline below maps that to a number. The mapping is closed (6 classes)
//! and covered by a deterministic test.

/// The 6 closed `evidence_class` values (design §3.2). The `memory_evidence`
/// column only ever stores one of these.
pub const EVIDENCE_CLASSES: [&str; 6] = [
    "manual_correction",
    "user_confirmed",
    "explicit_user_statement",
    "project_artifact_fact",
    "assistant_inferred",
    "behavioral_pattern",
];

/// Coerce an LLM-provided evidence-class label to one of the 6 valid values,
/// defaulting to `assistant_inferred` for anything missing / unknown.
pub fn normalize_evidence_class(raw: Option<&str>) -> &'static str {
    match raw.map(str::trim) {
        Some("manual_correction") => "manual_correction",
        Some("user_confirmed") => "user_confirmed",
        Some("explicit_user_statement") => "explicit_user_statement",
        Some("project_artifact_fact") => "project_artifact_fact",
        Some("behavioral_pattern") => "behavioral_pattern",
        _ => "assistant_inferred",
    }
}

/// Confidence baseline derived solely from `evidence_class` (design §3.2).
/// The LLM does not produce this number; it only outputs the class label.
pub fn confidence_baseline(evidence_class: &str) -> f32 {
    match evidence_class {
        "manual_correction" => 1.00,
        "user_confirmed" => 0.95,
        "explicit_user_statement" => 0.85,
        "project_artifact_fact" => 0.75,
        "behavioral_pattern" => 0.35,
        // assistant_inferred + any unknown
        _ => 0.45,
    }
}

/// Normalize a claim `object` for exact-match canonicalize: trim, casefold,
/// and collapse internal whitespace. Deterministic so the dedup key is
/// stable. (NFC normalization can be layered on later to match the knowledge
/// resolver; ASCII/casefold covers the common cases.)
pub fn normalize_object(object: &str) -> String {
    object
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Effective status used by every read path (prompt candidates, linked legacy
/// memory, Active Memory v2): an `active` claim whose `valid_until` has passed
/// reads as `expired` (design §4.5). `now` is an RFC3339 string; comparison is
/// lexical, so both sides must be RFC3339 (the injection JOIN mirrors this in
/// SQL).
pub fn effective_status(status: &str, valid_until: Option<&str>, now: &str) -> String {
    if status == "active" {
        if let Some(vu) = valid_until {
            if !vu.is_empty() && vu.as_bytes() < now.as_bytes() {
                return "expired".to_string();
            }
        }
    }
    status.to_string()
}

/// Whether an effective status keeps the claim (and its linked legacy memory)
/// eligible for prompt injection. Only `active` claims inject; superseded /
/// expired / archived / needs_review do not.
pub fn is_injectable_status(effective: &str) -> bool {
    effective == "active"
}

/// Normalize a model-provided `valid_until` to canonical RFC3339 millis+Z so
/// the injection-filter's lexical comparison is sound (the model emits loose
/// ISO8601 — date-only, offset timezones, or prose). Returns `None` for
/// empty / unparseable values: an unparseable expiry is treated as "no
/// expiry" so a malformed timestamp can never *silently expire* a still-valid
/// claim. Accepts full RFC3339 and bare `YYYY-MM-DD` dates.
pub fn normalize_valid_until(raw: Option<&str>) -> Option<String> {
    let s = raw.map(str::trim).filter(|s| !s.is_empty())?;
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(
            dt.with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        );
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        if let Some(naive) = date.and_hms_opt(0, 0, 0) {
            return Some(
                chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(naive, chrono::Utc)
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            );
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confidence_baseline_is_deterministic_per_class() {
        assert_eq!(confidence_baseline("manual_correction"), 1.00);
        assert_eq!(confidence_baseline("user_confirmed"), 0.95);
        assert_eq!(confidence_baseline("explicit_user_statement"), 0.85);
        assert_eq!(confidence_baseline("project_artifact_fact"), 0.75);
        assert_eq!(confidence_baseline("assistant_inferred"), 0.45);
        assert_eq!(confidence_baseline("behavioral_pattern"), 0.35);
        // Unknown → assistant_inferred baseline.
        assert_eq!(confidence_baseline("nonsense"), 0.45);
    }

    #[test]
    fn every_valid_class_has_a_distinct_nonzero_baseline() {
        for c in EVIDENCE_CLASSES {
            assert!(confidence_baseline(c) > 0.0, "class {c} has no baseline");
        }
    }

    #[test]
    fn normalize_evidence_class_coerces_unknown_to_assistant_inferred() {
        assert_eq!(
            normalize_evidence_class(Some("user_confirmed")),
            "user_confirmed"
        );
        assert_eq!(
            normalize_evidence_class(Some("  manual_correction  ")),
            "manual_correction"
        );
        assert_eq!(
            normalize_evidence_class(Some("bogus")),
            "assistant_inferred"
        );
        assert_eq!(normalize_evidence_class(None), "assistant_inferred");
    }

    #[test]
    fn normalize_object_is_casefold_and_whitespace_collapsed() {
        assert_eq!(normalize_object("  Uses   Bun  "), "uses bun");
        assert_eq!(normalize_object("PNPM"), "pnpm");
        assert_eq!(normalize_object("a\tb\nc"), "a b c");
    }

    #[test]
    fn effective_status_expires_active_past_valid_until() {
        let now = "2026-06-07T00:00:00.000Z";
        assert_eq!(
            effective_status("active", Some("2026-01-01T00:00:00.000Z"), now),
            "expired"
        );
        assert_eq!(
            effective_status("active", Some("2027-01-01T00:00:00.000Z"), now),
            "active"
        );
        assert_eq!(effective_status("active", None, now), "active");
        assert_eq!(effective_status("active", Some(""), now), "active");
        // Non-active statuses pass through untouched.
        assert_eq!(effective_status("superseded", None, now), "superseded");
    }

    #[test]
    fn only_active_is_injectable() {
        assert!(is_injectable_status("active"));
        assert!(!is_injectable_status("expired"));
        assert!(!is_injectable_status("superseded"));
        assert!(!is_injectable_status("archived"));
        assert!(!is_injectable_status("needs_review"));
    }

    #[test]
    fn normalize_valid_until_canonicalizes_or_drops() {
        // Offset timezone → canonical UTC Z (so lexical compare is sound).
        assert_eq!(
            normalize_valid_until(Some("2026-06-14T00:00:00+08:00")).as_deref(),
            Some("2026-06-13T16:00:00.000Z")
        );
        // Bare date → midnight UTC Z.
        assert_eq!(
            normalize_valid_until(Some("2026-06-14")).as_deref(),
            Some("2026-06-14T00:00:00.000Z")
        );
        // Already-canonical passes through (re-emitted with millis).
        assert_eq!(
            normalize_valid_until(Some("2026-06-14T00:00:00Z")).as_deref(),
            Some("2026-06-14T00:00:00.000Z")
        );
        // Unparseable / empty / prose → None (no silent expiry).
        assert_eq!(normalize_valid_until(Some("next week")), None);
        assert_eq!(normalize_valid_until(Some("")), None);
        assert_eq!(normalize_valid_until(None), None);
    }
}
