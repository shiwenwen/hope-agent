//! Gates 2 (pre-LLM) and 5 (post-LLM lint) of the five-gate waterfall.
//!
//! Both gates are deterministic, cheap, and run regardless of any user
//! override of `REVIEW_SYSTEM` — they are the structural floor that the
//! prompt cannot lower. Gate 2 turns the LLM call into a no-op when the
//! conversation shape is obviously not skill-worthy. Gate 5 sanity-checks
//! the body the model produced before it hits disk.

use std::collections::HashSet;

use super::config::SkillsAutoReviewConfig;

/// Reason name written into `ReviewReport::rationale` / `learning_events`.
/// Stable identifiers so the UI can localize them and humans can grep logs.
pub const REASON_TOO_FEW_MESSAGES: &str = "pre_gate_too_few_messages";
pub const REASON_DISCARD_BLACKLIST: &str = "pre_gate_discard_blacklist";
pub const REASON_SESSION_RECAP: &str = "post_lint_session_recap";
pub const REASON_STEP_COUNT: &str = "post_lint_step_count_out_of_range";
pub const REASON_TOO_ABSTRACT: &str = "post_lint_too_abstract";
pub const REASON_SESSION_ARTIFACT_NAME: &str = "post_lint_session_artifact_name";

/// Result of `pre_gate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreGateOutcome {
    pub allow: bool,
    pub reason: Option<String>,
    pub hit: Option<String>,
}

impl PreGateOutcome {
    fn allow() -> Self {
        Self {
            allow: true,
            reason: None,
            hit: None,
        }
    }

    fn deny(reason: &str, hit: Option<String>) -> Self {
        Self {
            allow: false,
            reason: Some(reason.to_string()),
            hit,
        }
    }
}

/// Gate 2: cheap pre-LLM checks. Returns `allow=false` to short-circuit the
/// pipeline before any model call.
///
/// `message_count` is the number of role-bearing entries in the trimmed
/// transcript (the same count the prompt would see). `conv_keys` is the
/// pre-tokenized transcript — callers tokenize once and share with the
/// dedup-block builder so we don't walk the conversation twice.
/// `discard_topics` is an unordered list of `(id, topic_text)` pairs
/// the user has discarded in the configured blacklist window. `topic_text`
/// is the language-rich representation: `description` if captured at
/// delete time, else the kebab `id` (which may not share a language
/// with the transcript).
pub fn pre_gate(
    cfg: &SkillsAutoReviewConfig,
    message_count: usize,
    conv_keys: &HashSet<String>,
    discard_topics: &[(String, String)],
) -> PreGateOutcome {
    if message_count < cfg.min_message_count {
        return PreGateOutcome::deny(REASON_TOO_FEW_MESSAGES, None);
    }
    if cfg.discard_blacklist_days > 0 && !discard_topics.is_empty() && !conv_keys.is_empty() {
        for (id, topic) in discard_topics {
            let topic_keys = tokenize(topic);
            if topic_keys.is_empty() {
                continue;
            }
            // Overlap coefficient (|A ∩ B| / min(|A|, |B|)) — asks
            // "does the conversation cover most of the rejected
            // topic's keywords", which is the right semantic for a
            // blacklist hit. Jaccard alone underweights short topic
            // strings against long transcripts.
            if overlap_coefficient(conv_keys, &topic_keys) >= 0.3 {
                return PreGateOutcome::deny(REASON_DISCARD_BLACKLIST, Some(id.clone()));
            }
        }
    }
    PreGateOutcome::allow()
}

/// Result of `post_lint`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostLintOutcome {
    pub allow: bool,
    pub reason: Option<String>,
    pub detail: Option<String>,
}

impl PostLintOutcome {
    fn allow() -> Self {
        Self {
            allow: true,
            reason: None,
            detail: None,
        }
    }

    fn deny(reason: &str, detail: Option<String>) -> Self {
        Self {
            allow: false,
            reason: Some(reason.to_string()),
            detail,
        }
    }
}

/// Markers that strongly indicate the body is just paraphrasing this
/// conversation rather than describing a class of work.
const SESSION_RECAP_MARKERS: &[&str] = &[
    "this conversation",
    "as we just discussed",
    "as discussed above",
    "in the prior message",
    "今天",
    "本次",
    "本对话",
    "上面",
    "刚才",
    "刚刚",
    "这次",
    "在我们的对话中",
];

/// Tokens that are a strong "this is a class-level skill" signal when the
/// body contains them — concrete commands, file paths, API names.
fn has_concrete_token(body: &str) -> bool {
    // command-line invocations, file paths, identifiers with dots, function
    // calls — any one of these is enough to clear the abstract floor.
    let lower = body.to_ascii_lowercase();
    if body.contains('`') {
        return true;
    }
    if body.contains("```") {
        return true;
    }
    if body.contains('/')
        && (lower.contains(".rs")
            || lower.contains(".ts")
            || lower.contains(".py")
            || lower.contains(".md")
            || lower.contains(".json")
            || lower.contains(".sh")
            || lower.contains(".tsx"))
    {
        return true;
    }
    // simple "looks like a command" heuristic: word followed by --flag or
    // word(...). Avoid regex deps — scan once.
    let mut prev_word = false;
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_alphanumeric() || c == '_' {
            prev_word = true;
        } else {
            if prev_word && c == '(' {
                return true;
            }
            if prev_word && c == ' ' {
                if let Some(&n) = chars.peek() {
                    if n == '-' {
                        return true;
                    }
                }
            }
            prev_word = false;
        }
    }
    false
}

/// Count Markdown ordered-list ("1.") and unordered-list ("- " / "* ")
/// items inside the obvious "steps" portion of the body. We are generous:
/// any list item anywhere in the body counts — the goal is to reject
/// bodies that have zero or hundreds of steps, not to parse Markdown
/// precisely.
fn step_count(body: &str) -> usize {
    let mut n = 0usize;
    for line in body.lines() {
        let t = line.trim_start();
        if t.starts_with("- ") || t.starts_with("* ") || t.starts_with("+ ") {
            n += 1;
            continue;
        }
        // ordered list "1. ", "12. "
        let mut chars = t.chars();
        let mut saw_digit = false;
        loop {
            match chars.next() {
                Some(c) if c.is_ascii_digit() => saw_digit = true,
                Some('.') if saw_digit => {
                    if let Some(' ') = chars.next() {
                        n += 1;
                    }
                    break;
                }
                _ => break,
            }
        }
    }
    n
}

/// Hard-coded markers that are a tell-tale "session artifact" in a name
/// claimed to be class-level. Case-insensitive substring match.
const SESSION_NAME_MARKERS: &[&str] = &[
    "-today",
    "-current",
    "-this",
    "-now",
    "-tmp",
    "-temp",
    "fix-issue",
    "investigate-",
];

/// Gate 5: deterministic body lint. `class_level_name` is whatever the
/// model self-reported in the JSON decision — we cross-check it against
/// the actual skill_id shape.
pub fn post_lint(
    cfg: &SkillsAutoReviewConfig,
    skill_id: &str,
    body: &str,
    class_level_name: bool,
) -> PostLintOutcome {
    // Step count window.
    let steps = step_count(body);
    if steps < cfg.min_steps || steps > cfg.max_steps {
        return PostLintOutcome::deny(
            REASON_STEP_COUNT,
            Some(format!(
                "steps={} min={} max={}",
                steps, cfg.min_steps, cfg.max_steps
            )),
        );
    }

    // Session-recap markers.
    let lower = body.to_ascii_lowercase();
    let mut recap_hits = 0usize;
    let mut first_hit: Option<String> = None;
    for marker in SESSION_RECAP_MARKERS {
        let m = marker.to_ascii_lowercase();
        if lower.contains(&m) {
            recap_hits += 1;
            if first_hit.is_none() {
                first_hit = Some((*marker).to_string());
            }
            if recap_hits >= cfg.session_recap_threshold && cfg.session_recap_threshold > 0 {
                return PostLintOutcome::deny(REASON_SESSION_RECAP, first_hit);
            }
        }
    }

    // Concrete token presence — a body with zero commands / paths / fences
    // is almost always abstract advice the model wrote on the fly.
    if !has_concrete_token(body) {
        return PostLintOutcome::deny(REASON_TOO_ABSTRACT, None);
    }

    // Session-artifact name check applies when the model claims class-level.
    if class_level_name {
        let lid = skill_id.to_ascii_lowercase();
        for marker in SESSION_NAME_MARKERS {
            if lid.contains(marker) {
                return PostLintOutcome::deny(
                    REASON_SESSION_ARTIFACT_NAME,
                    Some((*marker).to_string()),
                );
            }
        }
        // Pure-digit suffix like "fix-1234" — sign of a ticket number.
        if let Some(last) = lid.rsplit('-').next() {
            if !last.is_empty() && last.chars().all(|c| c.is_ascii_digit()) {
                return PostLintOutcome::deny(
                    REASON_SESSION_ARTIFACT_NAME,
                    Some(format!("trailing digits: {}", last)),
                );
            }
        }
    }

    PostLintOutcome::allow()
}

// ── Shared text helpers ─────────────────────────────────────────────────

/// Word-bag Jaccard. Whitespace + simple punctuation split, ASCII-lowered.
/// Single-character tokens are dropped (too noisy in CJK splits).
pub fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count() as f32;
    let union = a.union(b).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

/// Overlap coefficient: `|A ∩ B| / min(|A|, |B|)`. Better than Jaccard
/// for "does the smaller set sit inside the larger one?" — used by
/// the discard blacklist where a short skill description vs. a long
/// transcript would otherwise dilute the match.
pub fn overlap_coefficient(a: &HashSet<String>, b: &HashSet<String>) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count() as f32;
    let smaller = a.len().min(b.len()) as f32;
    if smaller == 0.0 {
        0.0
    } else {
        inter / smaller
    }
}

/// Lightweight tokenizer. ASCII words split on whitespace + punctuation.
/// CJK runs are split into bigrams to give the Jaccard something useful
/// even when there are no spaces.
pub fn tokenize(s: &str) -> HashSet<String> {
    let mut out: HashSet<String> = HashSet::new();
    let mut current = String::new();
    let mut cjk_run: Vec<char> = Vec::new();
    for c in s.chars() {
        if is_cjk(c) {
            cjk_run.push(c);
            if !current.is_empty() {
                push_token(&mut out, &current);
                current.clear();
            }
        } else if c.is_alphanumeric() || c == '_' {
            current.push(c.to_ascii_lowercase());
            if !cjk_run.is_empty() {
                flush_cjk(&mut out, &cjk_run);
                cjk_run.clear();
            }
        } else {
            if !current.is_empty() {
                push_token(&mut out, &current);
                current.clear();
            }
            if !cjk_run.is_empty() {
                flush_cjk(&mut out, &cjk_run);
                cjk_run.clear();
            }
        }
    }
    if !current.is_empty() {
        push_token(&mut out, &current);
    }
    if !cjk_run.is_empty() {
        flush_cjk(&mut out, &cjk_run);
    }
    out
}

fn push_token(out: &mut HashSet<String>, tok: &str) {
    if tok.chars().count() >= 2 {
        out.insert(tok.to_string());
    }
}

fn flush_cjk(out: &mut HashSet<String>, run: &[char]) {
    if run.len() < 2 {
        return;
    }
    for w in run.windows(2) {
        out.insert(w.iter().collect());
    }
}

fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{4E00}'..='\u{9FFF}'      // CJK Unified Ideographs
        | '\u{3400}'..='\u{4DBF}'    // CJK Ext A
        | '\u{20000}'..='\u{2A6DF}'  // CJK Ext B
        | '\u{3040}'..='\u{30FF}'    // Hiragana + Katakana
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_default() -> SkillsAutoReviewConfig {
        SkillsAutoReviewConfig::default()
    }

    #[test]
    fn pre_gate_blocks_short_transcripts() {
        let cfg = cfg_default();
        let keys = tokenize("[user]: hi\n[assistant]: hi");
        let out = pre_gate(&cfg, 2, &keys, &[]);
        assert!(!out.allow);
        assert_eq!(out.reason.as_deref(), Some(REASON_TOO_FEW_MESSAGES));
    }

    #[test]
    fn pre_gate_allows_long_transcripts() {
        let cfg = cfg_default();
        let keys = tokenize("[user]: rust clippy warning E0382 moved value");
        let out = pre_gate(&cfg, 10, &keys, &[]);
        assert!(out.allow, "expected allow, got {:?}", out);
    }

    #[test]
    fn pre_gate_blocks_discard_blacklist_match() {
        let cfg = cfg_default();
        let keys = tokenize("[user]: 我想把我的猫从城市带回农村");
        let discards = vec![(
            "adult-cat-relocation-to-rural".to_string(),
            "为成年猫从城市搬到农村提供安置步骤与风险评估".to_string(),
        )];
        let out = pre_gate(&cfg, 8, &keys, &discards);
        assert!(!out.allow);
        assert_eq!(out.reason.as_deref(), Some(REASON_DISCARD_BLACKLIST));
    }

    #[test]
    fn pre_gate_discard_blacklist_off_when_zero_days() {
        let mut cfg = cfg_default();
        cfg.discard_blacklist_days = 0;
        let keys = tokenize("[user]: 我想把我的猫从城市带回农村");
        let out = pre_gate(
            &cfg,
            8,
            &keys,
            &[(
                "adult-cat-relocation-to-rural".to_string(),
                "搬猫".to_string(),
            )],
        );
        assert!(out.allow);
    }

    #[test]
    fn step_count_counts_ordered_and_unordered() {
        let body = r"
some text

- one
- two

1. step
2. step
   nested text
3. last
";
        assert_eq!(step_count(body), 5);
    }

    #[test]
    fn post_lint_rejects_session_recap_markers() {
        let cfg = cfg_default();
        let body =
            "## 步骤\n1. 第一步\n2. 第二步\n\n刚才用户提到的问题，今天的处理是这样的：见 `foo()`。";
        let out = post_lint(&cfg, "ok-name", body, true);
        assert!(!out.allow);
        assert_eq!(out.reason.as_deref(), Some(REASON_SESSION_RECAP));
    }

    #[test]
    fn post_lint_rejects_too_few_steps() {
        let cfg = cfg_default();
        let body = "no list items here, just prose with `code`";
        let out = post_lint(&cfg, "ok-name", body, true);
        assert!(!out.allow);
        assert_eq!(out.reason.as_deref(), Some(REASON_STEP_COUNT));
    }

    #[test]
    fn post_lint_rejects_overly_long_step_lists() {
        let cfg = cfg_default();
        let mut body = String::new();
        for i in 0..20 {
            body.push_str(&format!("{}. step\n", i + 1));
        }
        body.push_str("`code`");
        let out = post_lint(&cfg, "ok-name", &body, true);
        assert!(!out.allow);
        assert_eq!(out.reason.as_deref(), Some(REASON_STEP_COUNT));
    }

    #[test]
    fn post_lint_rejects_abstract_body() {
        let cfg = cfg_default();
        let body = "## 步骤\n- 先这样\n- 再那样\n- 最后这样";
        let out = post_lint(&cfg, "ok-name", body, true);
        assert!(!out.allow);
        assert_eq!(out.reason.as_deref(), Some(REASON_TOO_ABSTRACT));
    }

    #[test]
    fn post_lint_rejects_session_artifact_name() {
        let cfg = cfg_default();
        let body = "## Steps\n1. run `cargo check`\n2. read `Cargo.toml`\n3. fix";
        let out = post_lint(&cfg, "fix-issue-123", body, true);
        assert!(!out.allow);
        assert_eq!(out.reason.as_deref(), Some(REASON_SESSION_ARTIFACT_NAME));
    }

    #[test]
    fn post_lint_accepts_clean_body() {
        let cfg = cfg_default();
        let body = "## Steps\n1. Run `cargo clippy --all-targets`\n2. Read `crates/foo/src/lib.rs` and patch warnings\n3. Re-run with `--locked`";
        let out = post_lint(&cfg, "audit-rust-clippy-warnings", body, true);
        assert!(out.allow, "got {:?}", out);
    }

    #[test]
    fn tokenize_handles_cjk_bigrams() {
        let t = tokenize("把猫带回农村");
        assert!(t.contains("把猫"));
        assert!(t.contains("农村"));
    }

    #[test]
    fn jaccard_basic() {
        let a = tokenize("rust clippy warnings");
        let b = tokenize("rust clippy issues");
        assert!(jaccard(&a, &b) > 0.0);
        assert!(jaccard(&a, &b) < 1.0);
    }
}
