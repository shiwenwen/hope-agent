use super::connection::{authenticate, callback, End, Socket};
use super::*;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;

fn account() -> ChannelAccountConfig {
    serde_json::from_value(json!({"id":"account-a","channelId":"wecom","label":"fixture",
        "credentials":{"botId":"bot-a","secret":"synthetic-fixture-secret"},"settings":{"imReplyMode":"final"},
        "security":{"dmPolicy":"allowlist","userAllowlist":["alice"],"groupPolicy":"allowlist",
            "groups":{"group:room":{"requireMention":false}}}})).unwrap()
}
fn frame(chat: &str, message: &str, request: &str) -> Value {
    json!({"cmd":"aibot_msg_callback","headers":{"req_id":request},"body":{"aibotid":"bot-a",
        "msgid":message,"from":{"userid":"alice"},"chattype":chat,"chatid":"room","msgtype":"text",
        "text":{"content":"synthetic text"},"response_url":"https://invalid.example/private","secret":"synthetic-private-echo"}})
}
fn timing() -> Timing {
    Timing {
        ack: Duration::from_millis(50),
        heartbeat: Duration::from_secs(10),
        tick: Duration::from_millis(5),
        anchor: Duration::from_secs(60),
    }
}
async fn pair() -> (
    Socket,
    tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        tokio_tungstenite::accept_async(socket).await.unwrap()
    });
    let (client, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    (client, server.await.unwrap())
}
async fn read(server: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>) -> Value {
    let message = tokio::time::timeout(Duration::from_secs(2), server.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    serde_json::from_str(message.to_text().unwrap()).unwrap()
}
async fn emit(
    server: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    frame: Value,
) {
    server
        .send(tokio_tungstenite::tungstenite::Message::Text(
            frame.to_string().into(),
        ))
        .await
        .unwrap();
}
fn command(
    chat: &str,
    message: &str,
) -> (
    Outbound,
    oneshot::Receiver<std::result::Result<RichReplyReceipt, ReplyStreamError>>,
) {
    let (result, rx) = oneshot::channel();
    (
        Outbound {
            chat: chat.into(),
            message: message.into(),
            text: "answer".into(),
            result,
        },
        rx,
    )
}

#[test]
fn wecom_callback_is_text_only_scoped_and_drops_raw_secrets() {
    let a = account();
    let (dm, request) = callback(&a, &frame("single", "m1", "r1")).unwrap();
    assert_eq!(request, "r1");
    assert_eq!(dm.chat_id, "single:alice");
    assert!(dm.raw.is_null());
    assert!(!dm.was_mentioned);
    assert!(allowed(&a, &dm));
    let (group, _) = callback(&a, &frame("group", "m1", "r2")).unwrap();
    assert_eq!(group.chat_id, "group:room");
    assert!(allowed(&a, &group));
    let mut other = a.clone();
    other.id = "account-b".into();
    assert!(!allowed(&other, &group));
    for chat in ["unknown", "channel", ""] {
        assert!(callback(&a, &frame(chat, "m1", "r1")).is_none());
    }
    let mut bad = frame("single", "m1", "r1");
    bad["body"]["aibotid"] = json!("other-bot");
    assert!(callback(&a, &bad).is_none());
    bad = frame("single", "m1", "r1");
    bad["body"]["msgtype"] = json!("image");
    assert!(callback(&a, &bad).is_none());
    bad = frame("single", "m1", "r1");
    bad["body"]["text"]["content"] = json!("x".repeat(MAX_TEXT + 1));
    assert!(callback(&a, &bad).is_none());
}
#[test]
fn wecom_empty_allowlists_and_unverifiable_mentions_stay_closed() {
    let mut a = account();
    let (dm, _) = callback(&a, &frame("single", "m1", "r1")).unwrap();
    a.security.user_allowlist.clear();
    assert!(!allowed(&a, &dm));
    let (group, _) = callback(&a, &frame("group", "m1", "r2")).unwrap();
    a.security.groups.clear();
    assert!(!allowed(&a, &group));
    a.security
        .groups
        .insert("group:room".into(), Default::default());
    assert!(!allowed(&a, &group));
    a.security
        .groups
        .get_mut("group:room")
        .unwrap()
        .require_mention = Some(true);
    assert!(!allowed(&a, &group));
    a.security
        .groups
        .get_mut("group:room")
        .unwrap()
        .require_mention = Some(false);
    assert!(allowed(&a, &group));
    a.security
        .groups
        .get_mut("group:room")
        .unwrap()
        .group_policy = Some(GroupPolicy::Disabled);
    assert!(!allowed(&a, &group));
    a.security
        .groups
        .get_mut("group:room")
        .unwrap()
        .group_policy = None;
    a.security.group_policy = GroupPolicy::Disabled;
    assert!(!allowed(&a, &group));
}
#[tokio::test]
async fn wecom_auth_requires_exact_ack_and_never_echoes_provider_text() {
    let (mut client, mut server) = pair().await;
    let task = tokio::spawn(async move {
        authenticate(
            &mut client,
            &Credentials::parse(&account().credentials).unwrap(),
            Duration::from_secs(2),
        )
        .await
    });
    let auth = read(&mut server).await;
    assert_eq!(auth["cmd"], "aibot_subscribe");
    assert_eq!(auth["body"]["bot_id"], "bot-a");
    emit(
        &mut server,
        json!({"headers":{"req_id":"wrong"},"errcode":0}),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(!task.is_finished());
    emit(
        &mut server,
        json!({"headers":auth["headers"],"errcode":40001,"errmsg":"synthetic-private-echo"}),
    )
    .await;
    assert_eq!(task.await.unwrap(), Err(ConnectError::Auth));
}
#[tokio::test]
async fn wecom_reply_is_exact_acked_single_use_and_wrong_chat_cannot_consume_it() {
    let (mut client, mut server) = pair().await;
    let (tx, rx) = mpsc::channel(16);
    let (inbound, mut incoming) = mpsc::channel(16);
    let cancel = CancellationToken::new();
    let actor_cancel = cancel.clone();
    let mut actor = Actor::new(
        account(),
        inbound,
        rx,
        Arc::default(),
        actor_cancel,
        Arc::default(),
        [0; 32],
    );
    let task = tokio::spawn(async move {
        actor
            .connection(
                &mut client,
                Timing {
                    ack: Duration::from_secs(1),
                    ..timing()
                },
            )
            .await
    });
    emit(&mut server, frame("single", "m1", "r1")).await;
    let event = tokio::time::timeout(Duration::from_secs(1), incoming.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(event, InboundEvent::Message(_)));
    emit(&mut server, frame("single", "m1", "r1")).await;
    let (bad, bad_result) = command("single:other", "m1");
    tx.send(bad).await.unwrap();
    assert_eq!(
        bad_result.await.unwrap().err().unwrap().kind,
        ReplyStreamErrorKind::Expired
    );
    let (cmd, mut receipt) = command("single:alice", "m1");
    tx.send(cmd).await.unwrap();
    let reply = read(&mut server).await;
    assert_eq!(reply["headers"]["req_id"], "r1");
    assert_eq!(reply["body"]["stream"]["finish"], true);
    assert_eq!(reply["body"]["stream"]["content"], "answer");
    emit(
        &mut server,
        json!({"headers":{"req_id":"wrong"},"errcode":0}),
    )
    .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(10), &mut receipt)
            .await
            .is_err()
    );
    emit(&mut server, json!({"headers":{"req_id":"r1"},"errcode":0})).await;
    let receipt = receipt.await.unwrap().unwrap();
    assert_eq!(
        receipt.message_id,
        reply["body"]["stream"]["id"].as_str().unwrap()
    );
    let (again, result) = command("single:alice", "m1");
    tx.send(again).await.unwrap();
    assert_eq!(
        result.await.unwrap().err().unwrap().kind,
        ReplyStreamErrorKind::Expired
    );
    assert!(incoming.try_recv().is_err());
    cancel.cancel();
    assert_eq!(task.await.unwrap(), End::Cancelled);
}
#[tokio::test]
async fn wecom_ack_timeout_is_ambiguous_and_never_resends() {
    let (mut client, mut server) = pair().await;
    let (tx, rx) = mpsc::channel(16);
    let (inbound, mut incoming) = mpsc::channel(16);
    let mut actor = Actor::new(
        account(),
        inbound,
        rx,
        Arc::default(),
        CancellationToken::new(),
        Arc::default(),
        [0; 32],
    );
    let task = tokio::spawn(async move { actor.connection(&mut client, timing()).await });
    emit(&mut server, frame("single", "m1", "r1")).await;
    incoming.recv().await.unwrap();
    let (cmd, receipt) = command("single:alice", "m1");
    tx.send(cmd).await.unwrap();
    assert_eq!(read(&mut server).await["cmd"], "aibot_respond_msg");
    let failure = receipt.await.unwrap().err().unwrap();
    assert_eq!(failure.kind, ReplyStreamErrorKind::Ambiguous);
    assert!(!failure.to_string().contains("synthetic"));
    assert_eq!(task.await.unwrap(), End::Network);
}

#[tokio::test]
async fn wecom_heartbeat_uses_ping_and_exact_ack_then_cancels_cleanly() {
    let (mut client, mut server) = pair().await;
    let (_tx, rx) = mpsc::channel(16);
    let (inbound, _incoming) = mpsc::channel(16);
    let cancel = CancellationToken::new();
    let mut actor = Actor::new(
        account(),
        inbound,
        rx,
        Arc::default(),
        cancel.clone(),
        Arc::default(),
        [0; 32],
    );
    let task = tokio::spawn(async move {
        actor
            .connection(
                &mut client,
                Timing {
                    heartbeat: Duration::from_millis(10),
                    ack: Duration::from_secs(1),
                    ..timing()
                },
            )
            .await
    });
    let first = read(&mut server).await;
    assert_eq!(first["cmd"], "ping");
    emit(&mut server, json!({"headers":first["headers"],"errcode":0})).await;
    let next = read(&mut server).await;
    assert_eq!(next["cmd"], "ping");
    assert_ne!(first["headers"], next["headers"]);
    cancel.cancel();
    assert_eq!(task.await.unwrap(), End::Cancelled);
}

#[tokio::test]
async fn wecom_replaced_connection_does_not_fight_other_clients() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (client, mut server) = pair().await;
    let (_tx, rx) = mpsc::channel(16);
    let (inbound, _incoming) = mpsc::channel(16);
    let cancel = CancellationToken::new();
    let health = Arc::new(Mutex::new(ChannelHealth::default()));
    let actor = Actor::new(
        account(),
        inbound,
        rx,
        health.clone(),
        cancel.clone(),
        Arc::default(),
        [0; 32],
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let task = tokio::spawn(async move {
        actor
            .run(
                client,
                move || {
                    count.fetch_add(1, Ordering::SeqCst);
                    async { Err(ConnectError::Network) }
                },
                timing(),
            )
            .await
    });
    emit(
        &mut server,
        json!({"cmd":"aibot_event_callback","body":{"event":{"eventtype":"disconnected_event"}}}),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if health
                .lock()
                .unwrap()
                .error
                .as_deref()
                .is_some_and(|v| v.contains("replaced"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(!task.is_finished());
    cancel.cancel();
    task.await.unwrap();
    assert_eq!(health.lock().unwrap().probe_ok, Some(false));
}

#[tokio::test]
async fn wecom_network_reconnect_clears_old_anchors_and_keeps_callback_dedup() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (client, mut first) = pair().await;
    let (next, mut second) = pair().await;
    let next = Arc::new(tokio::sync::Mutex::new(Some(next)));
    let (tx, rx) = mpsc::channel(16);
    let (inbound, mut incoming) = mpsc::channel(16);
    let cancel = CancellationToken::new();
    let actor = Actor::new(
        account(),
        inbound,
        rx,
        Arc::default(),
        cancel.clone(),
        Arc::default(),
        [0; 32],
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let task = tokio::spawn(async move {
        actor
            .run(
                client,
                move || {
                    let next = next.clone();
                    count.fetch_add(1, Ordering::SeqCst);
                    async move { next.lock().await.take().ok_or(ConnectError::Network) }
                },
                timing(),
            )
            .await
    });
    emit(&mut first, frame("single", "old", "r-old")).await;
    incoming.recv().await.unwrap();
    first.close(None).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while calls.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await
        }
    })
    .await
    .unwrap();
    emit(&mut second, frame("single", "old", "r-old")).await;
    emit(&mut second, frame("single", "new", "r-new")).await;
    let InboundEvent::Message(message) =
        tokio::time::timeout(Duration::from_secs(2), incoming.recv())
            .await
            .unwrap()
            .unwrap()
    else {
        panic!("expected message")
    };
    assert_eq!(message.message_id, "new");
    let (old, receipt) = command("single:alice", "old");
    tx.send(old).await.unwrap();
    assert_eq!(
        receipt.await.unwrap().err().unwrap().kind,
        ReplyStreamErrorKind::Expired
    );
    cancel.cancel();
    task.await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn wecom_rejected_auth_reconnect_is_latched_without_repeated_attempts() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (client, mut server) = pair().await;
    let (_tx, rx) = mpsc::channel(16);
    let (inbound, _incoming) = mpsc::channel(16);
    let rejected: Rejections = Arc::default();
    let health = Arc::new(Mutex::new(ChannelHealth::default()));
    let cancel = CancellationToken::new();
    let actor = Actor::new(
        account(),
        inbound,
        rx,
        health.clone(),
        cancel.clone(),
        rejected.clone(),
        [4; 32],
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let task = tokio::spawn(async move {
        actor
            .run(
                client,
                move || {
                    count.fetch_add(1, Ordering::SeqCst);
                    async { Err(ConnectError::Auth) }
                },
                timing(),
            )
            .await
    });
    server.close(None).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while rejected.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await
        }
    })
    .await
    .unwrap();
    assert_eq!(rejected.lock().unwrap().get("account-a"), Some(&[4; 32]));
    assert_eq!(health.lock().unwrap().probe_ok, Some(false));
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    cancel.cancel();
    task.await.unwrap();
}

#[tokio::test]
async fn wecom_validation_reuses_cached_health_without_opening_another_connection() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let plugin = WeComPlugin::new();
    let credentials = Credentials::parse(&account().credentials).unwrap();
    let (tx, _rx) = mpsc::channel(1);
    let health = Arc::new(Mutex::new(ChannelHealth {
        probe_ok: Some(true),
        ..Default::default()
    }));
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    plugin.accounts.lock().unwrap().insert(
        "account-a".into(),
        Running {
            bot_id: credentials.bot_id().to_owned(),
            fingerprint: credentials.fingerprint(),
            commands: tx,
            health: health.clone(),
            cancel,
            task: tokio::spawn(async move { token.cancelled().await }),
        },
    );
    let calls = AtomicUsize::new(0);
    assert_eq!(
        plugin
            .validate_with(&credentials, || {
                calls.fetch_add(1, Ordering::SeqCst);
                async { Err(ConnectError::Network) }
            })
            .await
            .unwrap(),
        "WeCom"
    );
    health.lock().unwrap().probe_ok = Some(false);
    assert!(plugin
        .validate_with(&credentials, || {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Err(ConnectError::Network) }
        })
        .await
        .is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    plugin.stop_account("account-a").await.unwrap();
}

#[tokio::test]
async fn wecom_auth_accepts_only_its_success_ack_and_ignores_pre_auth_callbacks() {
    let (mut client, mut server) = pair().await;
    let task = tokio::spawn(async move {
        authenticate(
            &mut client,
            &Credentials::parse(&account().credentials).unwrap(),
            Duration::from_secs(2),
        )
        .await
    });
    let auth = read(&mut server).await;
    emit(
        &mut server,
        frame("single", "before-auth", "callback-before-auth"),
    )
    .await;
    server
        .send(tokio_tungstenite::tungstenite::Message::Pong(
            vec![1, 2].into(),
        ))
        .await
        .unwrap();
    emit(&mut server, json!({"headers":auth["headers"],"errcode":0})).await;
    assert_eq!(task.await.unwrap(), Ok(()));
}

#[tokio::test]
async fn wecom_unsupported_content_and_proactive_targets_fail_before_a_send() {
    let plugin = WeComPlugin::new();
    let mut target = ReplyStreamTarget {
        account_id: "account-a".into(),
        chat_id: "single:alice".into(),
        chat_type: ChatType::Dm,
        thread_id: None,
        reply_to_message_id: Some("m1".into()),
        recipient_user_id: None,
        recipient_tenant_id: None,
    };
    let mut reply = RichReply {
        markdown: "ok".into(),
        media: vec![],
        buttons: vec![],
    };
    target.reply_to_message_id = None;
    assert_eq!(
        plugin.reply(&target, &reply).await.err().unwrap().kind,
        ReplyStreamErrorKind::InvalidTarget
    );
    target.reply_to_message_id = Some("m1".into());
    reply.markdown = "字".repeat(MAX_TEXT / 3 + 1);
    assert_eq!(
        plugin.reply(&target, &reply).await.err().unwrap().kind,
        ReplyStreamErrorKind::InvalidContent
    );
    reply.markdown = "ok".into();
    reply.buttons = vec![vec![InlineButton {
        text: "action".into(),
        callback_data: Some("synthetic".into()),
        url: None,
    }]];
    assert_eq!(
        plugin.reply(&target, &reply).await.err().unwrap().kind,
        ReplyStreamErrorKind::InvalidContent
    );
    reply.buttons.clear();
    reply.media = vec![OutboundMedia {
        media_type: MediaType::Document,
        data: MediaData::Bytes(vec![1, 2, 3]),
        caption: None,
    }];
    assert_eq!(
        plugin.reply(&target, &reply).await.err().unwrap().kind,
        ReplyStreamErrorKind::InvalidContent
    );
}

#[tokio::test]
async fn wecom_cancelled_queued_reply_does_not_mutate_or_consume_its_anchor() {
    let (mut client, mut server) = pair().await;
    let (tx, rx) = mpsc::channel(16);
    let (inbound, mut incoming) = mpsc::channel(16);
    let cancel = CancellationToken::new();
    let mut actor = Actor::new(
        account(),
        inbound,
        rx,
        Arc::default(),
        cancel.clone(),
        Arc::default(),
        [0; 32],
    );
    let task = tokio::spawn(async move {
        actor
            .connection(
                &mut client,
                Timing {
                    ack: Duration::from_secs(1),
                    ..timing()
                },
            )
            .await
    });
    emit(&mut server, frame("single", "m1", "r1")).await;
    incoming.recv().await.unwrap();
    let (mut aborted, receipt) = command("single:alice", "m1");
    aborted.text = "cancelled-message".into();
    drop(receipt);
    tx.send(aborted).await.unwrap();
    let (next, receipt) = command("single:alice", "m1");
    tx.send(next).await.unwrap();
    let body = read(&mut server).await;
    assert_eq!(body["body"]["stream"]["content"], "answer");
    emit(&mut server, json!({"headers":{"req_id":"r1"},"errcode":0})).await;
    assert!(receipt.await.unwrap().is_ok());
    cancel.cancel();
    assert_eq!(task.await.unwrap(), End::Cancelled);
}

#[tokio::test]
async fn wecom_rotated_secret_validation_cannot_replace_a_running_bot() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let plugin = WeComPlugin::new();
    let credentials = Credentials::parse(&account().credentials).unwrap();
    let (tx, _rx) = mpsc::channel(1);
    let health = Arc::new(Mutex::new(ChannelHealth {
        probe_ok: Some(true),
        ..Default::default()
    }));
    let cancel = CancellationToken::new();
    let worker_cancel = cancel.clone();
    plugin.accounts.lock().unwrap().insert(
        "account-a".into(),
        Running {
            bot_id: credentials.bot_id().to_owned(),
            fingerprint: credentials.fingerprint(),
            commands: tx,
            health: health.clone(),
            cancel: cancel.clone(),
            task: tokio::spawn(async move { worker_cancel.cancelled().await }),
        },
    );
    let calls = AtomicUsize::new(0);
    let rotated = Credentials::parse(
        &json!({"botId":credentials.bot_id(),"secret":"synthetic-rotated-secret"}),
    )
    .unwrap();
    assert_ne!(rotated.fingerprint(), credentials.fingerprint());
    let error = plugin
        .validate_with(&rotated, || {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Err(ConnectError::Network) }
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Stop this WeCom bot"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(health.lock().unwrap().probe_ok, Some(true));
    assert!(!cancel.is_cancelled());
    // A different Bot ID must not inherit this account's cached validation.
    let other = Credentials::parse(
        &json!({"botId":"different-synthetic-bot","secret":"synthetic-other-secret"}),
    )
    .unwrap();
    assert!(plugin
        .validate_with(&other, || {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Err(ConnectError::Network) }
        })
        .await
        .is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(!cancel.is_cancelled());
    plugin.stop_account("account-a").await.unwrap();
}
