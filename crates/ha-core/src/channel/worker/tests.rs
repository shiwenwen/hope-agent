use super::streaming::*;
use crate::channel::types::*;

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
        max_message_length: Some(4096),
        supports_card_stream,
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
        select_stream_preview_transport(&ChatType::Dm, &caps(true, true, false)),
        Some(StreamPreviewTransport::Draft)
    );
    assert_eq!(
        select_stream_preview_transport(&ChatType::Group, &caps(true, true, false)),
        Some(StreamPreviewTransport::Message)
    );
}

#[test]
fn select_preview_transport_prefers_card_in_groups_when_supported() {
    // Feishu group: no draft, has edit, has card stream → Card.
    assert_eq!(
        select_stream_preview_transport(&ChatType::Group, &caps(false, true, true)),
        Some(StreamPreviewTransport::Card)
    );
}

#[test]
fn select_preview_transport_prefers_card_in_dm_without_draft() {
    // Feishu DM: no draft, has edit, has card stream → Card (since Draft
    // is unavailable, Card is the next-best preview path).
    assert_eq!(
        select_stream_preview_transport(&ChatType::Dm, &caps(false, true, true)),
        Some(StreamPreviewTransport::Card)
    );
}

#[test]
fn select_preview_transport_keeps_draft_when_dm_supports_both() {
    // If a channel ever supports both Draft and Card streaming, Draft
    // wins in DMs (Telegram-style animated preview is still preferable).
    assert_eq!(
        select_stream_preview_transport(&ChatType::Dm, &caps(true, true, true)),
        Some(StreamPreviewTransport::Draft)
    );
}

#[test]
fn select_preview_transport_falls_back_to_message_when_card_disabled() {
    // Existing 11 non-Feishu channels: no card stream, may have edit.
    assert_eq!(
        select_stream_preview_transport(&ChatType::Group, &caps(false, true, false)),
        Some(StreamPreviewTransport::Message)
    );
}

#[test]
fn select_preview_transport_returns_none_when_no_preview_path_available() {
    assert_eq!(
        select_stream_preview_transport(&ChatType::Group, &caps(false, false, false)),
        None
    );
}

#[test]
fn draft_error_fallback_matches_unsupported_api_responses() {
    let err = "sendMessageDraft failed (404): method sendMessageDraft not found";
    assert!(should_fallback_from_draft_error(err));
}
