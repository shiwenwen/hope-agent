use super::dispatcher::merge_preview_round_texts;
use super::slash::render_options_help_text;
use super::streaming::*;
use async_trait::async_trait;
use ha_core::channel::traits::ChannelReplyStream;
use ha_core::channel::types::*;
use ha_core::chat_engine::RoundOutput;
use tokio::time::{Duration, Instant};

fn caps(
    supports_draft: bool,
    supports_edit: bool,
    supports_card_stream: bool,
) -> ChannelCapabilities {
    ChannelCapabilities {
        chat_types: vec![ChatType::Dm, ChatType::Group, ChatType::Forum],
        supports_polls: false,
        supports_reactions: false,
        supports_draft,
        supports_edit,
        supports_unsend: false,
        supports_reply: true,
        supports_threads: true,
        supports_media: Vec::new(),
        supports_typing: true,
        supports_buttons: false,
        streaming_preview_max_bytes: Some(4096),
        supports_card_stream,
        native_reply: None,
    }
}

fn target(chat_type: ChatType) -> ReplyStreamTarget {
    ReplyStreamTarget {
        account_id: "acc".to_string(),
        chat_id: "chat".to_string(),
        chat_type,
        thread_id: None,
        reply_to_message_id: Some("incoming".to_string()),
        recipient_user_id: Some("user".to_string()),
        recipient_tenant_id: Some("tenant".to_string()),
    }
}

fn native_caps(update_mode: ReplyStreamUpdateMode) -> NativeReplyCapabilities {
    NativeReplyCapabilities {
        preview_chat_types: vec![ChatType::Dm],
        final_chat_types: vec![ChatType::Dm],
        update_mode,
        requires_reply_anchor: true,
        requires_recipient_user_id: true,
        requires_recipient_tenant_id: true,
        supports_task_updates: true,
        supports_plan_updates: true,
        supports_blocks: true,
        embedded_media_types: vec![MediaType::Photo],
        refresh_after_secs: Some(5),
        max_snapshot_chars: None,
        max_delta_chars: Some(4),
    }
}

#[test]
fn extract_text_delta_reads_content_field() {
    let event = r#"{"type":"text_delta","content":"hello"}"#;
    assert_eq!(extract_text_delta(event).as_deref(), Some("hello"));
}

#[test]
fn extract_text_delta_keeps_legacy_text_field_compatibility() {
    let event = r#"{"type":"text_delta","text":"hello"}"#;
    assert_eq!(extract_text_delta(event).as_deref(), Some("hello"));
}

#[test]
fn select_preview_transport_prefers_draft_only_for_private_chats() {
    assert_eq!(
        select_stream_preview_transport(&target(ChatType::Dm), &caps(true, true, false), true),
        Some(StreamPreviewTransport::Draft)
    );
    assert_eq!(
        select_stream_preview_transport(&target(ChatType::Group), &caps(true, true, false), true),
        Some(StreamPreviewTransport::Message)
    );
}

#[test]
fn select_preview_transport_prefers_card_in_groups_when_supported() {
    // Feishu group: no draft, has edit, has card stream → Card.
    assert_eq!(
        select_stream_preview_transport(&target(ChatType::Group), &caps(false, true, true), true),
        Some(StreamPreviewTransport::Card)
    );
}

#[test]
fn select_preview_transport_prefers_card_in_dm_without_draft() {
    // Feishu DM: no draft, has edit, has card stream → Card (since Draft
    // is unavailable, Card is the next-best preview path).
    assert_eq!(
        select_stream_preview_transport(&target(ChatType::Dm), &caps(false, true, true), true),
        Some(StreamPreviewTransport::Card)
    );
}

#[test]
fn select_preview_transport_keeps_draft_when_dm_supports_both() {
    // If a channel ever supports both Draft and Card streaming, Draft
    // wins in DMs (Telegram-style animated preview is still preferable).
    assert_eq!(
        select_stream_preview_transport(&target(ChatType::Dm), &caps(true, true, true), true),
        Some(StreamPreviewTransport::Draft)
    );
}

#[test]
fn select_preview_transport_falls_back_to_message_when_card_disabled() {
    // Existing 11 non-Feishu channels: no card stream, may have edit.
    assert_eq!(
        select_stream_preview_transport(&target(ChatType::Group), &caps(false, true, false), true),
        Some(StreamPreviewTransport::Message)
    );
}

#[test]
fn select_preview_transport_returns_none_when_no_preview_path_available() {
    assert_eq!(
        select_stream_preview_transport(&target(ChatType::Group), &caps(false, false, false), true),
        None
    );
}

#[test]
fn native_preview_has_priority_only_for_a_complete_target() {
    let mut capabilities = caps(true, true, true);
    capabilities.native_reply = Some(native_caps(ReplyStreamUpdateMode::Append));
    assert!(matches!(
        select_stream_preview_transport(&target(ChatType::Dm), &capabilities, true),
        Some(StreamPreviewTransport::Native { .. })
    ));

    let mut missing_tenant = target(ChatType::Dm);
    missing_tenant.recipient_tenant_id = None;
    assert_eq!(
        select_stream_preview_transport(&missing_tenant, &capabilities, true),
        Some(StreamPreviewTransport::Draft),
    );
}

#[test]
fn native_open_claim_never_overwrites_a_terminal_cancel() {
    let selected = std::sync::atomic::AtomicU8::new(NATIVE_SELECTED);
    assert!(try_begin_native_open(&selected));
    assert_eq!(
        selected.load(std::sync::atomic::Ordering::Acquire),
        NATIVE_OPENING
    );

    let cancelled = std::sync::atomic::AtomicU8::new(NATIVE_TERMINAL);
    assert!(!try_begin_native_open(&cancelled));
    assert_eq!(
        cancelled.load(std::sync::atomic::Ordering::Acquire),
        NATIVE_TERMINAL
    );
}

#[tokio::test]
async fn native_abort_waits_for_existing_aborting_state_without_overwriting_it() {
    let state = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(NATIVE_ABORTING));
    let session: SharedNativeReplySession = std::sync::Arc::new(tokio::sync::Mutex::new(None));
    let preview = PreviewHandle::Native {
        session,
        state: state.clone(),
    };
    let completing_state = state.clone();
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        completing_state.store(NATIVE_AMBIGUOUS, std::sync::atomic::Ordering::Release);
    });

    assert!(!abort_native_preview(&preview, ReplyAbortReason::Failed).await);
    assert_eq!(
        state.load(std::sync::atomic::Ordering::Acquire),
        NATIVE_AMBIGUOUS
    );
}

struct PanicAbortStream;

#[async_trait]
impl ChannelReplyStream for PanicAbortStream {
    async fn push(
        &mut self,
        _frame: &ReplyStreamFrame,
    ) -> std::result::Result<(), ReplyStreamError> {
        Ok(())
    }

    async fn commit(
        self: Box<Self>,
        _final_reply: &RichReply,
    ) -> std::result::Result<RichReplyReceipt, ReplyStreamError> {
        unreachable!("not used by abort panic test")
    }

    async fn abort(
        self: Box<Self>,
        _reason: ReplyAbortReason,
    ) -> std::result::Result<(), ReplyStreamError> {
        panic!("synthetic adapter panic")
    }
}

#[tokio::test]
async fn native_abort_adapter_panic_becomes_ambiguous() {
    let state = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(NATIVE_ACTIVE));
    let session: SharedNativeReplySession =
        std::sync::Arc::new(tokio::sync::Mutex::new(Some(Box::new(PanicAbortStream))));
    let preview = PreviewHandle::Native {
        session,
        state: state.clone(),
    };

    assert!(!abort_native_preview(&preview, ReplyAbortReason::Failed).await);
    assert_eq!(
        state.load(std::sync::atomic::Ordering::Acquire),
        NATIVE_AMBIGUOUS
    );
}

#[test]
fn late_append_mirror_skips_native_but_snapshot_can_start() {
    let mut capabilities = caps(false, true, false);
    capabilities.native_reply = Some(native_caps(ReplyStreamUpdateMode::Append));
    assert_eq!(
        select_stream_preview_transport(&target(ChatType::Dm), &capabilities, false),
        Some(StreamPreviewTransport::Message),
    );

    capabilities.native_reply = Some(native_caps(ReplyStreamUpdateMode::Snapshot));
    assert!(matches!(
        select_stream_preview_transport(&target(ChatType::Dm), &capabilities, false),
        Some(StreamPreviewTransport::Native { .. })
    ));
}

#[test]
fn append_frame_acknowledges_only_the_emitted_utf8_safe_prefix() {
    let capabilities = native_caps(ReplyStreamUpdateMode::Append);
    let mut state = NativeFrameState::default();
    let frame = build_native_frame(
        "你好世界继续",
        &state,
        &capabilities,
        ReplyStreamPhase::Generating,
    );
    assert_eq!(frame.markdown_delta, "你好世界");
    assert_eq!(frame.markdown_snapshot, "你好世界");
    acknowledge_native_frame(&mut state, &frame, capabilities.update_mode);
    assert_eq!(state.acknowledged_bytes, "你好世界".len());

    let next = build_native_frame(
        "你好世界继续",
        &state,
        &capabilities,
        ReplyStreamPhase::Finalizing,
    );
    assert_eq!(next.markdown_delta, "继续");
    assert_eq!(next.revision, 2);
    assert_eq!(next.plan_title.as_deref(), Some("正在整理结果"));
}

#[test]
fn snapshot_frame_caps_chinese_and_emoji_by_chars_and_acks_snapshot_bytes() {
    let mut capabilities = native_caps(ReplyStreamUpdateMode::Snapshot);
    capabilities.max_snapshot_chars = Some(4);
    let mut state = NativeFrameState::default();
    let frame = build_native_frame(
        "你🙂好世界",
        &state,
        &capabilities,
        ReplyStreamPhase::Generating,
    );
    assert_eq!(frame.markdown_snapshot, "你🙂好世");
    assert_eq!(frame.markdown_delta, "你🙂好世");
    acknowledge_native_frame(&mut state, &frame, capabilities.update_mode);
    assert_eq!(state.acknowledged_bytes, "你🙂好世".len());
}

#[test]
fn safe_task_snapshot_never_copies_arguments_or_results() {
    let mut tracker = SafeTaskTracker::default();
    assert!(tracker.observe(
        r#"{"type":"tool_call","call_id":"secret-call","name":"web_fetch","arguments":"token=secret"}"#,
    ));
    assert!(tracker.observe(
        r#"{"type":"tool_result","call_id":"secret-call","is_error":false,"duration_ms":9,"result":"private file contents"}"#,
    ));
    let task = tracker.tasks.first().expect("task");
    assert!(!task.id.contains("secret-call"));
    assert_eq!(task.title, "web_fetch");
    assert_eq!(task.status, ReplyStreamTaskStatus::Complete);
    assert_eq!(task.details.as_deref(), Some("工具已完成（耗时 9 毫秒）"));
    let visible = format!("{} {:?}", task.title, task.details);
    assert!(!visible.contains("token=secret"));
    assert!(!visible.contains("private file contents"));
}

#[test]
fn draft_error_fallback_matches_unsupported_api_responses() {
    let err = "sendMessageDraft failed (404): method sendMessageDraft not found";
    assert!(should_fallback_from_draft_error(err));
}

/// Split-streaming detects round boundaries by string-matching the
/// emitted `tool_call` event. `serde_json` defaults to `BTreeMap` (no
/// `preserve_order`), so JSON keys serialize alphabetically and `type`
/// lands mid-string — `contains("\"type\":\"tool_call\"")` works,
/// `starts_with` would silently miss every event. Lock the contract
/// here so a future preserve_order flag flip surfaces in CI.
#[test]
fn tool_call_event_contains_anchor_for_split_streaming_boundary() {
    let event = serde_json::json!({
        "type": "tool_call",
        "call_id": "c1",
        "name": "send_attachment",
        "arguments": "{}",
    });
    let s = serde_json::to_string(&event).unwrap();
    assert!(
        s.contains("\"type\":\"tool_call\""),
        "split-streaming round-boundary check would miss this: {s}"
    );
    assert!(
        !s.starts_with("{\"type\""),
        "if this fires, BTreeMap key ordering changed; review streaming.rs guard: {s}"
    );
}

#[test]
fn stream_preview_outcome_default_reports_zero_finalized_rounds() {
    let outcome = StreamPreviewOutcome::default();
    assert!(outcome.preview.is_none());
    assert_eq!(
        outcome.finalized_rounds, 0,
        "default outcome must signal `dispatcher should ship every round`"
    );
}

#[test]
fn stream_preview_flush_schedule_starts_fast_then_uses_safe_cadence() {
    let start = Instant::now();
    let mut schedule = StreamPreviewFlushSchedule::new(start);

    assert!(!schedule.should_flush(
        true,
        true,
        start + STREAM_PREVIEW_FIRST_FLUSH_DELAY - Duration::from_millis(1)
    ));
    assert!(schedule.should_flush(true, true, start + STREAM_PREVIEW_FIRST_FLUSH_DELAY));
    assert!(!schedule.should_flush(false, true, start + Duration::from_secs(10)));
    assert!(!schedule.should_flush(true, false, start + Duration::from_secs(10)));

    let first_flush = start + STREAM_PREVIEW_FIRST_FLUSH_DELAY;
    schedule.mark_flushed(first_flush);
    assert!(!schedule.should_flush(
        true,
        true,
        first_flush + STREAM_PREVIEW_FLUSH_INTERVAL - Duration::from_millis(1)
    ));
    assert!(schedule.should_flush(true, true, first_flush + STREAM_PREVIEW_FLUSH_INTERVAL));
}

#[test]
fn append_preview_round_text_inserts_line_break_between_rounds() {
    let mut accumulated = "我把头像文件直接发给你。".to_string();
    append_preview_round_text(&mut accumulated, "已发送。", true);
    assert_eq!(accumulated, "我把头像文件直接发给你。\n已发送。");
}

#[test]
fn append_preview_round_text_keeps_same_round_byte_exact() {
    let mut accumulated = "hello".to_string();
    append_preview_round_text(&mut accumulated, " world", false);
    assert_eq!(accumulated, "hello world");
}

#[test]
fn merge_preview_round_texts_uses_same_line_break_contract() {
    let rounds = vec![
        RoundOutput {
            text: "我把头像文件直接发给你。".to_string(),
            ..RoundOutput::default()
        },
        RoundOutput::default(),
        RoundOutput {
            text: "已发送。".to_string(),
            ..RoundOutput::default()
        },
    ];
    assert_eq!(
        merge_preview_round_texts(&rounds),
        "我把头像文件直接发给你。\n已发送。"
    );
}

// ── preview_carried_full_text decision matrix ────────────────────────
//
// Locks in the contract that the stream task uses to decide whether the
// preview transport already shipped the round's full text or whether the
// finalize path must fall back to chunked `send_text_chunks`. Skipping
// that check on a "preview ran but silently dropped" outcome is exactly
// the high-severity Codex finding from 2026-05-06: stream task
// incremented `finalized_rounds`, dispatcher skipped the round, full
// narration was lost.

#[test]
fn preview_carries_text_for_message_when_message_exists_and_fits() {
    assert!(preview_carried_full_text(
        &StreamPreviewTransport::Message,
        "hello world",
        11,
        Some("msg-1"),
        None,
        4096,
    ));
}

#[test]
fn preview_does_not_carry_text_for_message_when_oversized() {
    // The pre-final round narration grew past Telegram's 4096 cap. Even
    // though a preview message exists, the latest edits were silently
    // dropped by `build_stream_preview_payload`. The stream task MUST
    // chunk-send so the user sees the full text.
    assert!(!preview_carried_full_text(
        &StreamPreviewTransport::Message,
        "long",
        4097,
        Some("msg-1"),
        None,
        4096,
    ));
}

#[test]
fn preview_does_not_carry_text_for_message_when_no_message_was_created() {
    // First text_delta already exceeded max_msg_len, so no preview
    // message ever opened. Without the fallback the round vanishes.
    assert!(!preview_carried_full_text(
        &StreamPreviewTransport::Message,
        "any",
        100,
        None,
        None,
        4096,
    ));
}

#[test]
fn preview_carries_text_for_card_when_session_active_and_under_cardkit_cap() {
    assert!(preview_carried_full_text(
        &StreamPreviewTransport::Card,
        "feishu narration",
        16,
        None,
        Some(false), // session active, not broken
        4096,
    ));
}

#[test]
fn preview_does_not_carry_text_for_card_when_session_broken() {
    // Mid-stream `update_card_element` failed → broken=true. Card
    // content lags; chunk-send the full round to recover.
    assert!(!preview_carried_full_text(
        &StreamPreviewTransport::Card,
        "narration",
        9,
        None,
        Some(true),
        4096,
    ));
}

#[test]
fn preview_does_not_carry_text_for_draft_ever() {
    // Drafts are typing indicators, not real messages. Even when the
    // accumulated text would fit a single send, we must chunk-and-send
    // so the user sees a real message in chat. (Chunk path correctly
    // becomes a single send for short text.)
    assert!(!preview_carried_full_text(
        &StreamPreviewTransport::Draft,
        "short",
        5,
        None,
        None,
        4096,
    ));
}

#[test]
fn preview_carries_empty_round_trivially() {
    // Zero-narration round (model went straight to tool_call). Nothing
    // to ship via either path; finalize_split_round still proceeds to
    // close the preview transport and deliver media.
    for transport in [
        StreamPreviewTransport::Message,
        StreamPreviewTransport::Card,
        StreamPreviewTransport::Draft,
    ] {
        assert!(
            preview_carried_full_text(&transport, "", 0, None, None, 4096),
            "empty accumulated should always count as 'carried' for {:?}",
            transport,
        );
    }
}

#[test]
fn options_help_text_lists_every_option_with_placeholder() {
    let text = render_options_help_text(
        "thinking",
        Some("<level>"),
        &[
            "off".into(),
            "low".into(),
            "medium".into(),
            "high".into(),
            "xhigh".into(),
        ],
    );
    assert!(
        text.starts_with("Usage: `/thinking <level>`"),
        "missing usage line: {text}"
    );
    for opt in ["off", "low", "medium", "high", "xhigh"] {
        assert!(
            text.contains(&format!("- `{opt}`")),
            "missing option {opt} in: {text}"
        );
    }
}

#[test]
fn options_help_text_falls_back_to_generic_placeholder() {
    let text = render_options_help_text("perm", None, &["yes".into(), "no".into()]);
    assert!(
        text.starts_with("Usage: `/perm <option>`"),
        "expected generic <option> placeholder: {text}"
    );
}
