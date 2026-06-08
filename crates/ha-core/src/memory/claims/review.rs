//! User-correction loop (Lucid Review, Phase 6 / design §5.2 §5.3).
//!
//! Turns owner-plane edits — approve / edit / reject / move-scope / pin /
//! mark-outdated / forget — into auditable mutations. Each action:
//!
//! 1. mutates the stored claim row via the [`super::store`] primitives (any →
//!    any status, unlike the resolver's active-gated path),
//! 2. writes a highest-priority `manual_correction` evidence row when it's a
//!    factual correction (design §5.3 — user corrections are authoritative),
//! 3. re-embeds when content changed so the next turn's Active Memory v2 /
//!    Context Pack recall reflects the new text,
//! 4. records a `user_correction` decision so every action has an audit trail,
//! 5. emits `memory:claim_changed` (+ `memory:review_required` when flagged) so
//!    the Dashboard refreshes.
//!
//! `update_claim` is the PATCH entry (edit / status / scope / pin); `forget_claim`
//! is the archive-or-delete entry. The diff → decision-type derivation lives in
//! the pure [`resolve_update`] so it's exhaustively unit-testable without a DB.

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::store::{
    add_correction_evidence, apply_claim_fields, claim_edit_state, forget_claim as store_forget,
    reembed_claim, ClaimEditState, ClaimFieldUpdate,
};

/// Salience set when a user pins a claim — above `PINNED_MIN_SALIENCE` (0.7) so
/// it enters the Context Pack static "Pinned Memory" segment.
const PIN_SALIENCE: f32 = 0.95;
/// Salience set when a user unpins — below the threshold (the claim stays
/// active for recall, just no longer force-injected).
const UNPIN_SALIENCE: f32 = 0.5;
/// Confidence a user confirmation / edit asserts (high, not absolute).
const USER_CONFIRMED_CONFIDENCE: f32 = 0.95;

/// Statuses a user may set (resolver-only `superseded` is excluded).
const ALLOWED_STATUS: [&str; 4] = ["active", "needs_review", "expired", "archived"];
const ALLOWED_SCOPE: [&str; 3] = ["global", "agent", "project"];

/// A partial claim update from the owner plane (PATCH semantics — every field
/// optional, `None` = leave unchanged). The frontend issues one logical action
/// per call, but the core tolerates multiple changed dimensions and records the
/// primary one in the audit log.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimUpdate {
    /// Defaulted so the HTTP layer can supply it from the URL path (the PATCH
    /// body omits it); the Tauri layer ships it inline.
    #[serde(default)]
    pub claim_id: String,
    pub content: Option<String>,
    pub subject: Option<String>,
    pub predicate: Option<String>,
    pub object: Option<String>,
    pub tags: Option<Vec<String>>,
    /// active | needs_review | expired | archived.
    pub status: Option<String>,
    /// global | agent | project — paired with `scope_id`.
    pub scope_type: Option<String>,
    pub scope_id: Option<String>,
    /// pin (true → salience boost) / unpin (false → reset).
    pub pinned: Option<bool>,
    /// Optional user-supplied reason, stored verbatim as the correction quote.
    pub note: Option<String>,
}

/// Outcome of a user correction (returned to the owner plane).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimActionOutcome {
    pub claim_id: String,
    /// The primary decision recorded: `approve` | `edit` | `reject` | `expire`
    /// | `move_scope` | `pin` | `unpin` | `flag` | `forget` |
    /// `forget_permanent` | `noop`.
    pub decision_type: String,
    pub changed: bool,
    /// `None` when nothing changed or the audit write failed (non-fatal).
    pub run_id: Option<String>,
}

/// The resolved effect of a [`ClaimUpdate`] against the current row — a pure
/// derivation (no DB) so the diff → decision mapping is unit-testable.
#[derive(Debug, Clone)]
struct ResolvedUpdate {
    fields: ClaimFieldUpdate,
    decision_type: &'static str,
    content_changed: bool,
    /// Effective scope after the update (for the evidence access-scope blob).
    scope_type: String,
    scope_id: Option<String>,
}

/// Derive the field update + primary decision type from `(before, req)`.
/// Priority when several dimensions change at once: status > scope > pin > edit.
/// Returns `decision_type = "noop"` when nothing actually changes.
fn resolve_update(before: &ClaimEditState, req: &ClaimUpdate) -> Result<ResolvedUpdate> {
    let mut fields = ClaimFieldUpdate::default();
    let mut content_changed = false;

    // ── Content + triple + tags (an "edit") ──
    if let Some(c) = req.content.as_ref() {
        let c = c.trim();
        if !c.is_empty() && c != before.content {
            fields.content = Some(c.to_string());
            content_changed = true;
        }
    }
    if let Some(v) = req.subject.as_ref() {
        let v = v.trim();
        if !v.is_empty() && v != before.subject {
            fields.subject = Some(v.to_string());
        }
    }
    if let Some(v) = req.predicate.as_ref() {
        let v = v.trim();
        if !v.is_empty() && v != before.predicate {
            fields.predicate = Some(v.to_string());
        }
    }
    if let Some(v) = req.object.as_ref() {
        let v = v.trim();
        if !v.is_empty() && v != before.object {
            fields.object = Some(v.to_string());
        }
    }
    if let Some(v) = req.tags.as_ref() {
        if *v != before.tags {
            fields.tags = Some(v.clone());
        }
    }
    let is_edit = content_changed
        || fields.subject.is_some()
        || fields.predicate.is_some()
        || fields.object.is_some()
        || fields.tags.is_some();

    // ── Status ──
    let mut status_target: Option<String> = None;
    if let Some(s) = req.status.as_ref() {
        if !ALLOWED_STATUS.contains(&s.as_str()) {
            bail!("invalid status: {s}");
        }
        if *s != before.status {
            fields.status = Some(s.clone());
            status_target = Some(s.clone());
        }
    }

    // ── Scope move ──
    let mut scope_type = before.scope_type.clone();
    let mut scope_id = before.scope_id.clone();
    let mut scope_changed = false;
    if let Some(stype) = req.scope_type.as_ref() {
        if !ALLOWED_SCOPE.contains(&stype.as_str()) {
            bail!("invalid scope_type: {stype}");
        }
        let new_sid = if stype == "global" {
            None
        } else {
            let sid = req
                .scope_id
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            if sid.is_none() {
                bail!("scope_id required for scope_type {stype}");
            }
            sid
        };
        if *stype != before.scope_type || new_sid != before.scope_id {
            fields.scope = Some((stype.clone(), new_sid.clone()));
            scope_type = stype.clone();
            scope_id = new_sid;
            scope_changed = true;
        }
    }

    // ── Pin / unpin (salience) ──
    let mut pin_change: Option<bool> = None;
    if let Some(pin) = req.pinned {
        let is_pinned = before.salience >= crate::memory::dreaming::PINNED_MIN_SALIENCE;
        if pin != is_pinned {
            fields.salience = Some(if pin { PIN_SALIENCE } else { UNPIN_SALIENCE });
            pin_change = Some(pin);
        }
    }

    // ── Primary decision type (status > scope > pin > edit) ──
    let decision_type: &'static str = if let Some(s) = status_target.as_deref() {
        match s {
            "active" => "approve",
            "archived" => "reject",
            "expired" => "expire",
            "needs_review" => "flag",
            _ => "update",
        }
    } else if scope_changed {
        "move_scope"
    } else if let Some(pin) = pin_change {
        if pin {
            "pin"
        } else {
            "unpin"
        }
    } else if is_edit {
        "edit"
    } else {
        "noop"
    };

    // Approve / edit are user confirmations → mark the claim user_confirmed and
    // raise confidence (it now rests on an explicit human signal).
    if decision_type == "approve" || decision_type == "edit" {
        fields.confidence_source = Some("user_confirmed".to_string());
        if before.confidence < USER_CONFIRMED_CONFIDENCE {
            fields.confidence = Some(USER_CONFIRMED_CONFIDENCE);
        }
    }

    Ok(ResolvedUpdate {
        fields,
        decision_type,
        content_changed,
        scope_type,
        scope_id,
    })
}

/// A human-readable default evidence quote when the user gave no note.
fn default_quote(decision_type: &str) -> &'static str {
    match decision_type {
        "approve" => "User approved this claim",
        "edit" => "User edited this claim",
        "reject" => "User rejected this claim",
        "expire" => "User marked this claim outdated",
        "move_scope" => "User changed this claim's scope",
        "forget" => "User forgot this memory",
        "forget_permanent" => "User permanently deleted this memory",
        _ => "User updated this claim",
    }
}

/// Apply a partial owner-plane update to one claim (design §5.2). Returns a
/// `noop` outcome when the request changes nothing.
pub fn update_claim(req: ClaimUpdate) -> Result<ClaimActionOutcome> {
    let claim_id = req.claim_id.trim().to_string();
    if claim_id.is_empty() {
        bail!("claim_id is required");
    }
    let before =
        claim_edit_state(&claim_id)?.ok_or_else(|| anyhow!("claim not found: {claim_id}"))?;
    let resolved = resolve_update(&before, &req)?;

    if resolved.decision_type == "noop" {
        return Ok(ClaimActionOutcome {
            claim_id,
            decision_type: "noop".to_string(),
            changed: false,
            run_id: None,
        });
    }

    let changed = apply_claim_fields(&claim_id, &resolved.fields)?;

    // Re-embed when content changed so the next turn's recall reflects it
    // (best-effort — a stale vector self-heals on the next reembed job).
    if resolved.content_changed {
        if let Err(e) = reembed_claim(&claim_id) {
            app_warn!(
                "memory",
                "claims::review",
                "re-embed after edit failed for {}: {}",
                claim_id,
                e
            );
        }
    }

    // Factual corrections get a highest-priority evidence row; pin / unpin /
    // flag are not corrections of the fact, so they skip it.
    let decision_type = resolved.decision_type;
    let note_text = crate::util::non_empty_trim_or(req.note.as_deref(), default_quote(decision_type));
    if matches!(
        decision_type,
        "approve" | "edit" | "reject" | "expire" | "move_scope"
    ) {
        let ev_class = if decision_type == "approve" {
            "user_confirmed"
        } else {
            "manual_correction"
        };
        if let Err(e) = add_correction_evidence(
            &claim_id,
            &resolved.scope_type,
            resolved.scope_id.as_deref(),
            ev_class,
            note_text,
        ) {
            app_warn!(
                "memory",
                "claims::review",
                "evidence write failed for {}: {}",
                claim_id,
                e
            );
        }
    }

    let before_json = json!({
        "status": before.status,
        "content": before.content,
        "scope": { "type": before.scope_type, "id": before.scope_id },
        "salience": before.salience,
        "confidenceSource": before.confidence_source,
    });
    let after_json = json!({
        "status": resolved.fields.status,
        "content": resolved.fields.content,
        "scope": resolved.fields.scope.as_ref().map(|(t, i)| json!({ "type": t, "id": i })),
        "salience": resolved.fields.salience,
        "note": req.note,
    });
    let run_id = record_action(decision_type, &claim_id, note_text, before_json, after_json);

    emit_claim_changed(&claim_id, decision_type);
    if decision_type == "flag" {
        emit_review_required(&claim_id);
    }

    Ok(ClaimActionOutcome {
        claim_id,
        decision_type: decision_type.to_string(),
        changed,
        run_id,
    })
}

/// Forget a claim (design §5.3). `permanent=false` archives it (kept as an
/// audit trail, linked legacy memories stop injecting); `true` hard-deletes the
/// claim graph and any legacy memory it solely managed.
pub fn forget_claim(
    claim_id: &str,
    permanent: bool,
    note: Option<&str>,
) -> Result<ClaimActionOutcome> {
    let claim_id = claim_id.trim().to_string();
    if claim_id.is_empty() {
        bail!("claim_id is required");
    }
    let before =
        claim_edit_state(&claim_id)?.ok_or_else(|| anyhow!("claim not found: {claim_id}"))?;

    let decision_type = if permanent {
        "forget_permanent"
    } else {
        "forget"
    };
    let quote = crate::util::non_empty_trim_or(note, default_quote(decision_type));

    // Archive keeps evidence, so write the correction note BEFORE archiving;
    // permanent discards the graph, so adding evidence would be pointless.
    if !permanent {
        if let Err(e) = add_correction_evidence(
            &claim_id,
            &before.scope_type,
            before.scope_id.as_deref(),
            "manual_correction",
            quote,
        ) {
            app_warn!(
                "memory",
                "claims::review",
                "forget evidence write failed for {}: {}",
                claim_id,
                e
            );
        }
    }

    let existed = store_forget(&claim_id, permanent)?;
    if !existed {
        return Ok(ClaimActionOutcome {
            claim_id,
            decision_type: "noop".to_string(),
            changed: false,
            run_id: None,
        });
    }

    let before_json = json!({
        "status": before.status,
        "content": before.content,
        "scope": { "type": before.scope_type, "id": before.scope_id },
    });
    let after_json = json!({ "permanent": permanent, "note": note });
    let run_id = record_action(decision_type, &claim_id, quote, before_json, after_json);

    emit_claim_changed(&claim_id, decision_type);

    Ok(ClaimActionOutcome {
        claim_id,
        decision_type: decision_type.to_string(),
        changed: true,
        run_id,
    })
}

/// Record one user action in the durable decision log (best-effort — the
/// correction already succeeded; a missing audit row must never fail it).
fn record_action(
    decision_type: &str,
    claim_id: &str,
    rationale: &str,
    before: serde_json::Value,
    after: serde_json::Value,
) -> Option<String> {
    match crate::memory::dreaming::record_user_action(
        decision_type,
        claim_id,
        rationale,
        before,
        after,
    ) {
        Ok(id) => Some(id),
        Err(e) => {
            app_warn!(
                "memory",
                "claims::review",
                "audit write failed for {} ({}): {}",
                claim_id,
                decision_type,
                e
            );
            None
        }
    }
}

fn emit_claim_changed(claim_id: &str, action: &str) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            "memory:claim_changed",
            json!({ "claimId": claim_id, "action": action }),
        );
    }
}

fn emit_review_required(claim_id: &str) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit("memory:review_required", json!({ "claimId": claim_id }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> ClaimEditState {
        ClaimEditState {
            scope_type: "global".to_string(),
            scope_id: None,
            content: "User prefers dark mode".to_string(),
            subject: "user".to_string(),
            predicate: "prefers".to_string(),
            object: "dark mode".to_string(),
            tags: vec![],
            status: "needs_review".to_string(),
            salience: 0.5,
            confidence: 0.6,
            confidence_source: "derived".to_string(),
        }
    }

    fn req(claim_id: &str) -> ClaimUpdate {
        ClaimUpdate {
            claim_id: claim_id.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn approve_flips_to_active_and_confirms() {
        let before = base();
        let mut r = req("c1");
        r.status = Some("active".to_string());
        let resolved = resolve_update(&before, &r).unwrap();
        assert_eq!(resolved.decision_type, "approve");
        assert_eq!(resolved.fields.status.as_deref(), Some("active"));
        assert_eq!(
            resolved.fields.confidence_source.as_deref(),
            Some("user_confirmed")
        );
        assert_eq!(resolved.fields.confidence, Some(USER_CONFIRMED_CONFIDENCE));
    }

    #[test]
    fn reject_maps_to_archived() {
        let before = base();
        let mut r = req("c1");
        r.status = Some("archived".to_string());
        assert_eq!(resolve_update(&before, &r).unwrap().decision_type, "reject");
    }

    #[test]
    fn mark_outdated_maps_to_expire() {
        let before = base();
        let mut r = req("c1");
        r.status = Some("expired".to_string());
        assert_eq!(resolve_update(&before, &r).unwrap().decision_type, "expire");
    }

    #[test]
    fn edit_content_marks_changed_and_confirms() {
        let before = base();
        let mut r = req("c1");
        r.content = Some("User prefers light mode".to_string());
        let resolved = resolve_update(&before, &r).unwrap();
        assert_eq!(resolved.decision_type, "edit");
        assert!(resolved.content_changed);
        assert_eq!(
            resolved.fields.confidence_source.as_deref(),
            Some("user_confirmed")
        );
    }

    #[test]
    fn unchanged_content_is_noop() {
        let before = base();
        let mut r = req("c1");
        r.content = Some("  User prefers dark mode  ".to_string()); // trims to same
        assert_eq!(resolve_update(&before, &r).unwrap().decision_type, "noop");
    }

    #[test]
    fn empty_request_is_noop() {
        let before = base();
        assert_eq!(
            resolve_update(&before, &req("c1")).unwrap().decision_type,
            "noop"
        );
    }

    #[test]
    fn move_scope_to_project_sets_both_columns() {
        let before = base();
        let mut r = req("c1");
        r.scope_type = Some("project".to_string());
        r.scope_id = Some("proj-7".to_string());
        let resolved = resolve_update(&before, &r).unwrap();
        assert_eq!(resolved.decision_type, "move_scope");
        assert_eq!(
            resolved.fields.scope,
            Some(("project".to_string(), Some("proj-7".to_string())))
        );
        assert_eq!(resolved.scope_type, "project");
        assert_eq!(resolved.scope_id.as_deref(), Some("proj-7"));
    }

    #[test]
    fn move_scope_to_agent_without_id_errors() {
        let before = base();
        let mut r = req("c1");
        r.scope_type = Some("agent".to_string());
        assert!(resolve_update(&before, &r).is_err());
    }

    #[test]
    fn move_to_global_clears_scope_id() {
        let mut before = base();
        before.scope_type = "agent".to_string();
        before.scope_id = Some("ha-main".to_string());
        let mut r = req("c1");
        r.scope_type = Some("global".to_string());
        let resolved = resolve_update(&before, &r).unwrap();
        assert_eq!(resolved.decision_type, "move_scope");
        assert_eq!(resolved.fields.scope, Some(("global".to_string(), None)));
    }

    #[test]
    fn pin_boosts_salience_above_threshold() {
        let before = base(); // salience 0.5 → not pinned
        let mut r = req("c1");
        r.pinned = Some(true);
        let resolved = resolve_update(&before, &r).unwrap();
        assert_eq!(resolved.decision_type, "pin");
        assert!(resolved.fields.salience.unwrap() >= crate::memory::dreaming::PINNED_MIN_SALIENCE);
    }

    #[test]
    fn pin_when_already_pinned_is_noop() {
        let mut before = base();
        before.salience = 0.9; // already above threshold
        let mut r = req("c1");
        r.pinned = Some(true);
        assert_eq!(resolve_update(&before, &r).unwrap().decision_type, "noop");
    }

    #[test]
    fn unpin_drops_salience_below_threshold() {
        let mut before = base();
        before.salience = 0.9;
        let mut r = req("c1");
        r.pinned = Some(false);
        let resolved = resolve_update(&before, &r).unwrap();
        assert_eq!(resolved.decision_type, "unpin");
        assert!(resolved.fields.salience.unwrap() < crate::memory::dreaming::PINNED_MIN_SALIENCE);
    }

    #[test]
    fn status_takes_priority_over_edit() {
        let before = base();
        let mut r = req("c1");
        r.status = Some("active".to_string());
        r.content = Some("new content".to_string());
        // Both change, but status wins the primary decision label.
        assert_eq!(
            resolve_update(&before, &r).unwrap().decision_type,
            "approve"
        );
    }

    #[test]
    fn invalid_status_errors() {
        let before = base();
        let mut r = req("c1");
        r.status = Some("superseded".to_string());
        assert!(resolve_update(&before, &r).is_err());
    }
}
