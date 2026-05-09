//! Parse non-message Feishu events into [`InboundEvent`] variants.
//!
//! Maps the 9 event types that v0.2.0 surfaces (Phase B.1 of the飞书完整对齐
//! roadmap) onto the cross-channel event enum. Business behavior — sync to
//! `messages` table on edit/recall, BotLeft cleanup, auto-welcome on join —
//! is deferred to v0.3+ Phase B.2; the dispatcher currently logs each variant
//! and drops it.
//!
//! Reference:
//! <https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1>
//! and <https://open.feishu.cn/document/server-docs/group/chat-events>

use serde::Deserialize;
use tokio::sync::mpsc;

use crate::channel::types::{
    ChannelId, ChatType, EventCommon, InboundEvent, MembershipAction, MembershipEvent,
    ReactionEvent, ReadReceiptEvent, RecalledMessageEvent,
};

// ── Shared deserialization helpers ──────────────────────────────

#[derive(Debug, Deserialize)]
struct OpenIdHolder {
    open_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReaderIdHolder {
    /// Real reader open_id is wrapped one level deeper: `reader.reader_id.open_id`.
    reader_id: Option<OpenIdHolder>,
}

fn map_chat_type(s: Option<&str>) -> ChatType {
    match s {
        Some("p2p") => ChatType::Dm,
        // Feishu uses "group" for normal groups, "topic" for forum-style
        // chats. We don't have a Topic ChatType; collapse to Group.
        Some("group") | Some("topic") => ChatType::Group,
        _ => ChatType::Group,
    }
}

fn now_utc() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now()
}

fn warn_parse_failed(account_id: &str, event_type: &str, err: &serde_json::Error) {
    app_warn!(
        "channel",
        "feishu:inbound",
        "[{}] Failed to parse non-message event {}: {}",
        account_id,
        event_type,
        err
    );
}

// ── Reaction created / deleted ──────────────────────────────────

#[derive(Debug, Deserialize)]
struct ReactionPayload {
    message_id: Option<String>,
    /// Recent docs include `chat_id`; older payloads may omit it. We accept
    /// missing and fall back to empty (log-only handler shows what we got).
    chat_id: Option<String>,
    reaction_type: Option<ReactionTypeField>,
    /// The user who added/removed the reaction.
    user_id: Option<OpenIdHolder>,
}

#[derive(Debug, Deserialize)]
struct ReactionTypeField {
    emoji_type: Option<String>,
}

fn parse_reaction(event: serde_json::Value, account_id: &str, added: bool) -> Option<InboundEvent> {
    let p: ReactionPayload = match serde_json::from_value(event.clone()) {
        Ok(v) => v,
        Err(e) => {
            let label = if added {
                "im.message.reaction.created_v1"
            } else {
                "im.message.reaction.deleted_v1"
            };
            warn_parse_failed(account_id, label, &e);
            return None;
        }
    };
    let message_id = p.message_id?;
    let sender_id = p.user_id.and_then(|u| u.open_id).unwrap_or_default();
    let emoji = p
        .reaction_type
        .and_then(|t| t.emoji_type)
        .unwrap_or_default();
    Some(InboundEvent::Reaction(ReactionEvent {
        common: EventCommon {
            channel_id: ChannelId::Feishu,
            account_id: account_id.to_string(),
            chat_id: p.chat_id.unwrap_or_default(),
            // Feishu reaction events don't carry chat_type. Group is the
            // most common context; Dm reactions also exist but are rare.
            chat_type: ChatType::Group,
            timestamp: now_utc(),
            raw: event,
        },
        message_id,
        sender_id,
        emoji,
        added,
    }))
}

// ── Message recalled ────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct RecalledPayload {
    message_id: Option<String>,
    chat_id: Option<String>,
    /// "message_owner" | "chat_admin" — Feishu doesn't expose the recaller's
    /// open_id directly, so RecalledMessageEvent.recalled_by stays None.
    #[allow(dead_code)]
    recall_type: Option<String>,
}

fn parse_recalled(event: serde_json::Value, account_id: &str) -> Option<InboundEvent> {
    let p: RecalledPayload = match serde_json::from_value(event.clone()) {
        Ok(v) => v,
        Err(e) => {
            warn_parse_failed(account_id, "im.message.recalled_v1", &e);
            return None;
        }
    };
    let message_id = p.message_id?;
    Some(InboundEvent::MessageRecalled(RecalledMessageEvent {
        common: EventCommon {
            channel_id: ChannelId::Feishu,
            account_id: account_id.to_string(),
            chat_id: p.chat_id.unwrap_or_default(),
            chat_type: ChatType::Group,
            timestamp: now_utc(),
            raw: event,
        },
        message_id,
        recalled_by: None,
    }))
}

// ── Message read receipt ────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ReadPayload {
    reader: Option<ReaderIdHolder>,
    /// One read event may cover multiple messages — fan out as separate
    /// InboundEvents downstream so each message can be handled independently.
    message_id_list: Option<Vec<String>>,
}

fn parse_read_receipt_list(event: serde_json::Value, account_id: &str) -> Vec<InboundEvent> {
    let p: ReadPayload = match serde_json::from_value(event.clone()) {
        Ok(v) => v,
        Err(e) => {
            warn_parse_failed(account_id, "im.message.message_read_v1", &e);
            return Vec::new();
        }
    };
    let reader_id = p
        .reader
        .and_then(|r| r.reader_id)
        .and_then(|h| h.open_id)
        .unwrap_or_default();
    let messages = p.message_id_list.unwrap_or_default();
    messages
        .into_iter()
        .map(|message_id| {
            InboundEvent::ReadReceipt(ReadReceiptEvent {
                common: EventCommon {
                    channel_id: ChannelId::Feishu,
                    account_id: account_id.to_string(),
                    // Read receipt event doesn't carry chat_id.
                    chat_id: String::new(),
                    chat_type: ChatType::Group,
                    timestamp: now_utc(),
                    raw: event.clone(),
                },
                message_id,
                reader_id: reader_id.clone(),
            })
        })
        .collect()
}

// ── Chat member: user added / deleted ──────────────────────────

#[derive(Debug, Deserialize)]
struct ChatMemberUserPayload {
    chat_id: Option<String>,
    operator_id: Option<OpenIdHolder>,
    users: Option<Vec<UserItem>>,
}

#[derive(Debug, Deserialize)]
struct UserItem {
    user_id: Option<OpenIdHolder>,
}

fn parse_user_added_or_deleted(
    event: serde_json::Value,
    account_id: &str,
    added: bool,
) -> Vec<InboundEvent> {
    let p: ChatMemberUserPayload = match serde_json::from_value(event.clone()) {
        Ok(v) => v,
        Err(e) => {
            let label = if added {
                "im.chat.member.user.added_v1"
            } else {
                "im.chat.member.user.deleted_v1"
            };
            warn_parse_failed(account_id, label, &e);
            return Vec::new();
        }
    };
    let operator = p.operator_id.and_then(|h| h.open_id);
    let chat_id = p.chat_id.unwrap_or_default();
    p.users
        .unwrap_or_default()
        .into_iter()
        .filter_map(|u| u.user_id.and_then(|h| h.open_id))
        .map(|user_id| {
            let action = if added {
                MembershipAction::UserJoined {
                    user_id,
                    inviter_id: operator.clone(),
                }
            } else {
                MembershipAction::UserLeft {
                    user_id,
                    kicked_by: operator.clone(),
                }
            };
            InboundEvent::Membership(MembershipEvent {
                common: EventCommon {
                    channel_id: ChannelId::Feishu,
                    account_id: account_id.to_string(),
                    chat_id: chat_id.clone(),
                    chat_type: ChatType::Group,
                    timestamp: now_utc(),
                    raw: event.clone(),
                },
                action,
            })
        })
        .collect()
}

// ── Chat member: bot added / deleted ───────────────────────────

#[derive(Debug, Deserialize)]
struct ChatMemberBotPayload {
    chat_id: Option<String>,
    operator_id: Option<OpenIdHolder>,
}

fn parse_bot_added_or_deleted(
    event: serde_json::Value,
    account_id: &str,
    added: bool,
) -> Option<InboundEvent> {
    let p: ChatMemberBotPayload = match serde_json::from_value(event.clone()) {
        Ok(v) => v,
        Err(e) => {
            let label = if added {
                "im.chat.member.bot.added_v1"
            } else {
                "im.chat.member.bot.deleted_v1"
            };
            warn_parse_failed(account_id, label, &e);
            return None;
        }
    };
    let operator = p.operator_id.and_then(|h| h.open_id);
    let action = if added {
        MembershipAction::BotJoined { added_by: operator }
    } else {
        MembershipAction::BotLeft {
            removed_by: operator,
        }
    };
    Some(InboundEvent::Membership(MembershipEvent {
        common: EventCommon {
            channel_id: ChannelId::Feishu,
            account_id: account_id.to_string(),
            chat_id: p.chat_id.unwrap_or_default(),
            chat_type: ChatType::Group,
            timestamp: now_utc(),
            raw: event,
        },
        action,
    }))
}

// ── Chat lifecycle: created / disbanded ─────────────────────────

#[derive(Debug, Deserialize)]
struct ChatCreatedPayload {
    chat_id: Option<String>,
    /// Present in `im.chat.created_v1` ("p2p" | "group"); absent in
    /// `im.chat.disbanded_v1` (which doesn't need the type to log).
    chat_type: Option<String>,
}

fn parse_chat_created(event: serde_json::Value, account_id: &str) -> Option<InboundEvent> {
    let p: ChatCreatedPayload = match serde_json::from_value(event.clone()) {
        Ok(v) => v,
        Err(e) => {
            warn_parse_failed(account_id, "im.chat.created_v1", &e);
            return None;
        }
    };
    Some(InboundEvent::Membership(MembershipEvent {
        common: EventCommon {
            channel_id: ChannelId::Feishu,
            account_id: account_id.to_string(),
            chat_id: p.chat_id.unwrap_or_default(),
            chat_type: map_chat_type(p.chat_type.as_deref()),
            timestamp: now_utc(),
            raw: event,
        },
        action: MembershipAction::ChatCreated,
    }))
}

#[derive(Debug, Deserialize)]
struct ChatDisbandedPayload {
    chat_id: Option<String>,
}

fn parse_chat_disbanded(event: serde_json::Value, account_id: &str) -> Option<InboundEvent> {
    let p: ChatDisbandedPayload = match serde_json::from_value(event.clone()) {
        Ok(v) => v,
        Err(e) => {
            warn_parse_failed(account_id, "im.chat.disbanded_v1", &e);
            return None;
        }
    };
    Some(InboundEvent::Membership(MembershipEvent {
        common: EventCommon {
            channel_id: ChannelId::Feishu,
            account_id: account_id.to_string(),
            chat_id: p.chat_id.unwrap_or_default(),
            chat_type: ChatType::Group,
            timestamp: now_utc(),
            raw: event,
        },
        action: MembershipAction::ChatDisbanded,
    }))
}

// ── Public dispatch entry ───────────────────────────────────────

/// Try to parse a non-message Feishu `event_type` into [`InboundEvent`](s)
/// and forward each through `inbound_tx`. Returns `true` if the event_type
/// was recognized (regardless of whether the payload parsed cleanly —
/// malformed payloads warn-log and yield zero events). Returns `false` for
/// unknown event_types so the caller can fall through to its own debug log.
pub async fn try_dispatch_non_message(
    event_type: &str,
    event_data: serde_json::Value,
    account_id: &str,
    inbound_tx: &mpsc::Sender<InboundEvent>,
) -> bool {
    let events: Vec<InboundEvent> = match event_type {
        "im.message.reaction.created_v1" => parse_reaction(event_data, account_id, true)
            .into_iter()
            .collect(),
        "im.message.reaction.deleted_v1" => parse_reaction(event_data, account_id, false)
            .into_iter()
            .collect(),
        "im.message.recalled_v1" => parse_recalled(event_data, account_id).into_iter().collect(),
        "im.message.message_read_v1" => parse_read_receipt_list(event_data, account_id),
        "im.chat.member.user.added_v1" => parse_user_added_or_deleted(event_data, account_id, true),
        "im.chat.member.user.deleted_v1" => {
            parse_user_added_or_deleted(event_data, account_id, false)
        }
        "im.chat.member.bot.added_v1" => parse_bot_added_or_deleted(event_data, account_id, true)
            .into_iter()
            .collect(),
        "im.chat.member.bot.deleted_v1" => {
            parse_bot_added_or_deleted(event_data, account_id, false)
                .into_iter()
                .collect()
        }
        "im.chat.created_v1" => parse_chat_created(event_data, account_id)
            .into_iter()
            .collect(),
        "im.chat.disbanded_v1" => parse_chat_disbanded(event_data, account_id)
            .into_iter()
            .collect(),
        _ => return false,
    };

    for ev in events {
        if let Err(e) = inbound_tx.send(ev).await {
            app_warn!(
                "channel",
                "feishu:inbound",
                "[{}] Failed to forward non-message event ({}): {}",
                account_id,
                event_type,
                e
            );
            // Don't break — the channel is closed, but forward attempts on
            // remaining events are also no-ops; let them log too for
            // diagnosis.
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::types::MembershipAction;

    async fn dispatch_and_collect(
        event_type: &str,
        event: serde_json::Value,
    ) -> (bool, Vec<InboundEvent>) {
        let (tx, mut rx) = mpsc::channel(8);
        let recognized = try_dispatch_non_message(event_type, event, "feishu-acc1", &tx).await;
        drop(tx);
        let mut out = Vec::new();
        while let Some(ev) = rx.recv().await {
            out.push(ev);
        }
        (recognized, out)
    }

    #[tokio::test]
    async fn reaction_created_emits_added_true() {
        let event = serde_json::json!({
            "message_id": "om_x",
            "chat_id": "oc_x",
            "reaction_type": {"emoji_type": "SMILE"},
            "user_id": {"open_id": "ou_user1"}
        });
        let (recognized, events) =
            dispatch_and_collect("im.message.reaction.created_v1", event).await;
        assert!(recognized);
        assert_eq!(events.len(), 1);
        match &events[0] {
            InboundEvent::Reaction(r) => {
                assert!(r.added);
                assert_eq!(r.message_id, "om_x");
                assert_eq!(r.sender_id, "ou_user1");
                assert_eq!(r.emoji, "SMILE");
                assert_eq!(r.common.chat_id, "oc_x");
                assert_eq!(r.common.channel_id, ChannelId::Feishu);
            }
            other => panic!("expected Reaction, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn reaction_deleted_emits_added_false() {
        let event = serde_json::json!({
            "message_id": "om_x",
            "reaction_type": {"emoji_type": "THUMBSUP"},
            "user_id": {"open_id": "ou_user2"}
        });
        let (recognized, events) =
            dispatch_and_collect("im.message.reaction.deleted_v1", event).await;
        assert!(recognized);
        match &events[0] {
            InboundEvent::Reaction(r) => {
                assert!(!r.added);
                assert_eq!(r.emoji, "THUMBSUP");
            }
            _ => panic!("expected Reaction"),
        }
    }

    #[tokio::test]
    async fn message_recalled_no_recaller() {
        let event = serde_json::json!({
            "message_id": "om_recalled",
            "chat_id": "oc_x",
            "recall_type": "message_owner"
        });
        let (recognized, events) = dispatch_and_collect("im.message.recalled_v1", event).await;
        assert!(recognized);
        match &events[0] {
            InboundEvent::MessageRecalled(r) => {
                assert_eq!(r.message_id, "om_recalled");
                assert!(r.recalled_by.is_none());
                assert_eq!(r.common.chat_id, "oc_x");
            }
            _ => panic!("expected MessageRecalled"),
        }
    }

    #[tokio::test]
    async fn message_read_fans_out_per_message() {
        let event = serde_json::json!({
            "reader": {
                "reader_id": {"open_id": "ou_reader"},
                "read_time": "1700000000000"
            },
            "message_id_list": ["om_a", "om_b", "om_c"]
        });
        let (recognized, events) = dispatch_and_collect("im.message.message_read_v1", event).await;
        assert!(recognized);
        assert_eq!(events.len(), 3);
        let ids: Vec<&str> = events
            .iter()
            .map(|ev| match ev {
                InboundEvent::ReadReceipt(r) => r.message_id.as_str(),
                _ => panic!("expected ReadReceipt"),
            })
            .collect();
        assert_eq!(ids, vec!["om_a", "om_b", "om_c"]);
        if let InboundEvent::ReadReceipt(r) = &events[0] {
            assert_eq!(r.reader_id, "ou_reader");
        }
    }

    #[tokio::test]
    async fn user_added_emits_one_membership_per_user() {
        let event = serde_json::json!({
            "chat_id": "oc_g",
            "operator_id": {"open_id": "ou_admin"},
            "users": [
                {"user_id": {"open_id": "ou_alice"}},
                {"user_id": {"open_id": "ou_bob"}}
            ]
        });
        let (recognized, events) =
            dispatch_and_collect("im.chat.member.user.added_v1", event).await;
        assert!(recognized);
        assert_eq!(events.len(), 2);
        match &events[0] {
            InboundEvent::Membership(m) => match &m.action {
                MembershipAction::UserJoined {
                    user_id,
                    inviter_id,
                } => {
                    assert_eq!(user_id, "ou_alice");
                    assert_eq!(inviter_id.as_deref(), Some("ou_admin"));
                }
                other => panic!("expected UserJoined, got {:?}", other),
            },
            _ => panic!("expected Membership"),
        }
    }

    #[tokio::test]
    async fn user_deleted_emits_user_left() {
        let event = serde_json::json!({
            "chat_id": "oc_g",
            "operator_id": {"open_id": "ou_kicker"},
            "users": [
                {"user_id": {"open_id": "ou_kicked"}}
            ]
        });
        let (_, events) = dispatch_and_collect("im.chat.member.user.deleted_v1", event).await;
        match &events[0] {
            InboundEvent::Membership(m) => match &m.action {
                MembershipAction::UserLeft { user_id, kicked_by } => {
                    assert_eq!(user_id, "ou_kicked");
                    assert_eq!(kicked_by.as_deref(), Some("ou_kicker"));
                }
                other => panic!("expected UserLeft, got {:?}", other),
            },
            _ => panic!("expected Membership"),
        }
    }

    #[tokio::test]
    async fn bot_added_emits_bot_joined() {
        let event = serde_json::json!({
            "chat_id": "oc_new",
            "operator_id": {"open_id": "ou_inviter"}
        });
        let (_, events) = dispatch_and_collect("im.chat.member.bot.added_v1", event).await;
        match &events[0] {
            InboundEvent::Membership(m) => match &m.action {
                MembershipAction::BotJoined { added_by } => {
                    assert_eq!(added_by.as_deref(), Some("ou_inviter"));
                }
                other => panic!("expected BotJoined, got {:?}", other),
            },
            _ => panic!("expected Membership"),
        }
    }

    #[tokio::test]
    async fn bot_deleted_emits_bot_left() {
        let event = serde_json::json!({
            "chat_id": "oc_old",
            "operator_id": {"open_id": "ou_kicker"}
        });
        let (_, events) = dispatch_and_collect("im.chat.member.bot.deleted_v1", event).await;
        match &events[0] {
            InboundEvent::Membership(m) => match &m.action {
                MembershipAction::BotLeft { removed_by } => {
                    assert_eq!(removed_by.as_deref(), Some("ou_kicker"));
                }
                other => panic!("expected BotLeft, got {:?}", other),
            },
            _ => panic!("expected Membership"),
        }
    }

    #[tokio::test]
    async fn chat_created_maps_chat_type() {
        let event = serde_json::json!({
            "chat_id": "oc_brand_new",
            "chat_type": "group"
        });
        let (_, events) = dispatch_and_collect("im.chat.created_v1", event).await;
        match &events[0] {
            InboundEvent::Membership(m) => {
                assert!(matches!(m.action, MembershipAction::ChatCreated));
                assert_eq!(m.common.chat_id, "oc_brand_new");
                assert_eq!(m.common.chat_type, ChatType::Group);
            }
            _ => panic!("expected Membership"),
        }
    }

    #[tokio::test]
    async fn chat_disbanded_emits_disbanded() {
        let event = serde_json::json!({"chat_id": "oc_gone"});
        let (_, events) = dispatch_and_collect("im.chat.disbanded_v1", event).await;
        match &events[0] {
            InboundEvent::Membership(m) => {
                assert!(matches!(m.action, MembershipAction::ChatDisbanded));
                assert_eq!(m.common.chat_id, "oc_gone");
            }
            _ => panic!("expected Membership"),
        }
    }

    #[tokio::test]
    async fn unknown_event_type_returns_false() {
        let (recognized, events) =
            dispatch_and_collect("im.message.share_v1", serde_json::json!({})).await;
        assert!(!recognized);
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn malformed_reaction_payload_does_not_panic() {
        let event = serde_json::json!({"reaction_type": "not-an-object"});
        let (recognized, events) =
            dispatch_and_collect("im.message.reaction.created_v1", event).await;
        // Recognized as the right event_type, but no events emitted because
        // the payload couldn't deserialize cleanly.
        assert!(recognized);
        assert!(events.is_empty());
    }

    #[test]
    fn chat_type_mapping() {
        assert_eq!(map_chat_type(Some("p2p")), ChatType::Dm);
        assert_eq!(map_chat_type(Some("group")), ChatType::Group);
        assert_eq!(map_chat_type(Some("topic")), ChatType::Group);
        assert_eq!(map_chat_type(Some("unknown")), ChatType::Group);
        assert_eq!(map_chat_type(None), ChatType::Group);
    }
}
