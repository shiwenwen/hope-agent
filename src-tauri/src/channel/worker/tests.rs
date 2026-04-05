use super::streaming::*;
use crate::channel::types::*;

fn caps(supports_draft: bool, supports_edit: bool) -> ChannelCapabilities {
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
        max_message_length: Some(4096),
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
        select_stream_preview_transport(&ChatType::Dm, &caps(true, true)),
        Some(StreamPreviewTransport::Draft)
    );
    assert_eq!(
        select_stream_preview_transport(&ChatType::Group, &caps(true, true)),
        Some(StreamPreviewTransport::Message)
    );
}

#[test]
fn draft_error_fallback_matches_unsupported_api_responses() {
    let err = "sendMessageDraft failed (404): method sendMessageDraft not found";
    assert!(should_fallback_from_draft_error(err));
}
