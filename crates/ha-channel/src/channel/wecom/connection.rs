use super::*;
use futures_util::{SinkExt, StreamExt};
use lru::LruCache;
use serde_json::json;
use std::future::Future;
use std::num::NonZeroUsize;
use tokio::net::TcpStream;
use tokio::time::Instant;
use tokio_tungstenite::{
    client_async_tls_with_config,
    tungstenite::{protocol::WebSocketConfig, Message},
    MaybeTlsStream, WebSocketStream,
};

const ENDPOINT: &str = "wss://openws.work.weixin.qq.com";
pub(super) type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ConnectError {
    Auth,
    Network,
    Cancelled,
}

pub(super) struct Credentials {
    bot_id: String,
    secret: String,
}
impl Credentials {
    pub fn parse(value: &Value) -> Result<Self> {
        let bot_id = value
            .get("botId")
            .and_then(Value::as_str)
            .filter(|v| valid_id(v));
        let secret = value
            .get("secret")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty() && v.len() <= 4096 && !v.chars().any(char::is_control));
        match (bot_id, secret) {
            (Some(bot_id), Some(secret)) => Ok(Self {
                bot_id: bot_id.into(),
                secret: secret.into(),
            }),
            _ => Err(anyhow!("WeCom requires a valid Bot ID and Secret")),
        }
    }
    pub fn bot_id(&self) -> &str {
        &self.bot_id
    }

    pub fn fingerprint(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(self.bot_id.len().to_be_bytes());
        hash.update(self.bot_id.as_bytes());
        hash.update(self.secret.as_bytes());
        hash.finalize().into()
    }
}

pub(super) fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

pub(super) async fn connect(
    credentials: &Credentials,
    cancel: &CancellationToken,
) -> std::result::Result<Socket, ConnectError> {
    let operation = async {
        use ha_core::security::ssrf::{check_url, resolve_checked_destination, SsrfPolicy};
        // DNS is checked again immediately before pinning TCP. TLS still uses
        // the fixed original hostname; the handshake does not follow redirects.
        check_url("https://openws.work.weixin.qq.com", SsrfPolicy::Strict, &[])
            .await
            .map_err(|_| ConnectError::Network)?;
        let destinations =
            resolve_checked_destination("openws.work.weixin.qq.com", 443, SsrfPolicy::Strict, &[])
                .await
                .map_err(|_| ConnectError::Network)?;
        let mut tcp = None;
        for destination in destinations.into_iter().take(16) {
            if let Ok(Ok(stream)) =
                tokio::time::timeout(Duration::from_secs(2), TcpStream::connect(destination)).await
            {
                tcp = Some(stream);
                break;
            }
        }
        let tcp = tcp.ok_or(ConnectError::Network)?;
        let config = WebSocketConfig::default()
            .max_message_size(Some(65_536))
            .max_frame_size(Some(65_536))
            .max_write_buffer_size(131_072)
            .write_buffer_size(16_384);
        let (mut socket, _) = client_async_tls_with_config(ENDPOINT, tcp, Some(config), None)
            .await
            .map_err(|_| ConnectError::Network)?;
        authenticate(&mut socket, credentials, Duration::from_secs(5)).await?;
        Ok(socket)
    };
    tokio::select! {
        _ = cancel.cancelled() => Err(ConnectError::Cancelled),
        result = tokio::time::timeout(Duration::from_secs(15), operation) => result.unwrap_or(Err(ConnectError::Network)),
    }
}

pub(super) async fn authenticate(
    socket: &mut Socket,
    credentials: &Credentials,
    deadline: Duration,
) -> std::result::Result<(), ConnectError> {
    let request = uuid::Uuid::new_v4().to_string();
    let operation = async {
        socket
            .send(Message::Text(
                json!({"cmd":"aibot_subscribe","headers":{"req_id":request},
            "body":{"bot_id":credentials.bot_id,"secret":credentials.secret}})
                .to_string()
                .into(),
            ))
            .await
            .map_err(|_| ConnectError::Network)?;
        for _ in 0..32 {
            match socket.next().await {
                Some(Ok(Message::Text(text))) => {
                    let frame: Value =
                        serde_json::from_str(&text).map_err(|_| ConnectError::Network)?;
                    if frame.get("cmd").is_none()
                        && frame.pointer("/headers/req_id").and_then(Value::as_str)
                            == Some(&request)
                    {
                        return match frame.get("errcode").and_then(Value::as_i64) {
                            Some(0) => Ok(()),
                            Some(_) => Err(ConnectError::Auth),
                            None => Err(ConnectError::Network),
                        };
                    }
                }
                Some(Ok(Message::Ping(data))) => {
                    socket
                        .send(Message::Pong(data))
                        .await
                        .map_err(|_| ConnectError::Network)?;
                }
                Some(Ok(Message::Pong(_))) => {}
                _ => return Err(ConnectError::Network),
            }
        }
        Err(ConnectError::Network)
    };
    tokio::time::timeout(deadline, operation)
        .await
        .unwrap_or(Err(ConnectError::Network))
}

#[derive(Clone, Copy)]
pub(super) struct Timing {
    pub ack: Duration,
    pub heartbeat: Duration,
    pub tick: Duration,
    pub anchor: Duration,
}
impl Default for Timing {
    fn default() -> Self {
        Self {
            ack: Duration::from_secs(5),
            heartbeat: Duration::from_secs(30),
            tick: Duration::from_secs(1),
            anchor: Duration::from_secs(600),
        }
    }
}
struct Anchor {
    request: String,
    created: Instant,
}
struct Pending {
    request: String,
    id: String,
    deadline: Instant,
    result: oneshot::Sender<std::result::Result<RichReplyReceipt, ReplyStreamError>>,
}
#[derive(PartialEq, Eq, Debug)]
pub(super) enum End {
    Network,
    Cancelled,
    Replaced,
}

pub(super) struct Actor {
    account: ChannelAccountConfig,
    inbound: mpsc::Sender<InboundEvent>,
    commands: mpsc::Receiver<Outbound>,
    health: Arc<Mutex<ChannelHealth>>,
    cancel: CancellationToken,
    rejected: Rejections,
    fingerprint: [u8; 32],
    seen: LruCache<(String, String), Instant>,
    requests: LruCache<String, Instant>,
    last_denial: Option<Instant>,
}
impl Actor {
    pub fn new(
        account: ChannelAccountConfig,
        inbound: mpsc::Sender<InboundEvent>,
        commands: mpsc::Receiver<Outbound>,
        health: Arc<Mutex<ChannelHealth>>,
        cancel: CancellationToken,
        rejected: Rejections,
        fingerprint: [u8; 32],
    ) -> Self {
        Self {
            account,
            last_denial: None,
            inbound,
            commands,
            health,
            cancel,
            rejected,
            fingerprint,
            seen: LruCache::new(NonZeroUsize::new(4096).expect("positive")),
            requests: LruCache::new(NonZeroUsize::new(4096).expect("positive")),
        }
    }
    fn status(&self, ready: bool, error: Option<&str>) {
        *self.health.lock().expect("WeCom health") = ChannelHealth {
            is_running: true,
            probe_ok: Some(ready),
            error: error.map(str::to_owned),
            last_probe: Some(chrono::Utc::now().to_rfc3339()),
            bot_name: Some("WeCom".into()),
            ..Default::default()
        };
    }
    pub async fn run<C, F>(mut self, mut socket: Socket, reconnect: C, timing: Timing)
    where
        C: Fn() -> F,
        F: Future<Output = std::result::Result<Socket, ConnectError>>,
    {
        let mut attempt = 0;
        loop {
            self.status(true, None);
            let end = self.connection(&mut socket, timing).await;
            drop(socket); // Every disconnect invalidates all callback anchors.
            if end == End::Cancelled {
                break;
            }
            if end == End::Replaced {
                self.status(false, Some("WeCom connection replaced by another client; stop the other client and restart this account"));
                self.cancel.cancelled().await;
                break;
            }
            self.status(
                false,
                Some("WeCom disconnected; reconnecting with bounded backoff"),
            );
            loop {
                let delay = super::super::ws::backoff_duration(attempt);
                attempt = attempt.saturating_add(1);
                tokio::select! { _ = self.cancel.cancelled() => { self.status(false, Some("WeCom stopped")); return; }, _ = tokio::time::sleep(delay) => {} }
                let result = tokio::select! { _ = self.cancel.cancelled() => Err(ConnectError::Cancelled), result = reconnect() => result };
                match result {
                    Ok(next) => {
                        socket = next;
                        attempt = 0;
                        break;
                    }
                    Err(ConnectError::Cancelled) => {
                        self.status(false, Some("WeCom stopped"));
                        return;
                    }
                    Err(ConnectError::Auth) => {
                        self.rejected
                            .lock()
                            .expect("WeCom rejections")
                            .insert(self.account.id.clone(), self.fingerprint);
                        self.status(false, Some("WeCom authentication rejected; validate corrected credentials before retrying"));
                        self.cancel.cancelled().await;
                        self.status(false, Some("WeCom stopped"));
                        return;
                    }
                    Err(ConnectError::Network) => {}
                }
            }
        }
        self.status(false, Some("WeCom stopped"));
    }

    pub async fn connection(&mut self, socket: &mut Socket, timing: Timing) -> End {
        let mut anchors: HashMap<(String, String), Anchor> = HashMap::new();
        let mut pending: Option<Pending> = None;
        let mut heartbeat: Option<(String, Instant)> = None;
        let mut next_heartbeat = Instant::now() + timing.heartbeat;
        let mut tick = tokio::time::interval(timing.tick);
        let end = loop {
            tokio::select! {
                _ = self.cancel.cancelled() => break End::Cancelled,
                _ = tick.tick() => {
                    let now = Instant::now();
                    anchors.retain(|_, a| now.duration_since(a.created) < timing.anchor);
                    if pending.as_ref().is_some_and(|p| now >= p.deadline)
                        || heartbeat.as_ref().is_some_and(|(_, deadline)| now >= *deadline) { break End::Network; }
                    if heartbeat.is_none() && now >= next_heartbeat {
                        let id = uuid::Uuid::new_v4().to_string();
                        heartbeat = Some((id.clone(), now + timing.ack));
                        if !write(socket, json!({"cmd":"ping","headers":{"req_id":id}}), &self.cancel, timing.ack).await { break End::Network; }
                        next_heartbeat = Instant::now() + timing.heartbeat;
                    }
                }
                frame = socket.next() => {
                    match frame {
                        Some(Ok(Message::Text(text))) => {
                            let Ok(frame) = serde_json::from_str::<Value>(&text) else { break End::Network; };
                            if frame.get("cmd").is_none() {
                                let id = frame.pointer("/headers/req_id").and_then(Value::as_str);
                                if pending.as_ref().is_some_and(|p| Some(p.request.as_str()) == id) {
                                    let value = pending.take().expect("matching pending");
                                    let result = match frame.get("errcode").and_then(Value::as_i64) {
                                        Some(0) => Ok(RichReplyReceipt::text_only(value.id)),
                                        Some(_) => Err(error(ReplyStreamErrorKind::Rejected)),
                                        None => { let _ = value.result.send(Err(error(ReplyStreamErrorKind::Ambiguous))); break End::Network; }
                                    };
                                    let _ = value.result.send(result);
                                } else if heartbeat.as_ref().is_some_and(|(request,_)| Some(request.as_str()) == id) {
                                    if frame.get("errcode").and_then(Value::as_i64) != Some(0) { break End::Network; }
                                    heartbeat = None;
                                }
                            } else if frame.get("cmd").and_then(Value::as_str) == Some("aibot_event_callback")
                                && frame.pointer("/body/event/eventtype").and_then(Value::as_str) == Some("disconnected_event") { break End::Replaced; }
                            else if let Some((msg, request)) = callback(&self.account, &frame) {
                                if !allowed(&self.account, &msg) {
                                    if self.last_denial.is_none_or(|at| at.elapsed() >= Duration::from_secs(30)) {
                                        let safe_id = |value: &str| {
                                            let mut value = value.to_owned();
                                            if let Some(secret) = self.account.credentials.get("secret").and_then(Value::as_str).filter(|v| !v.is_empty()) {
                                                value = value.replace(secret, "[REDACTED]");
                                            }
                                            ha_core::logging::redact_sensitive(&value)
                                        };
                                        app_warn!("channel", "wecom", "WeCom policy denied account={} sender={} chat={} (no message content)", self.account.id, safe_id(&msg.sender_id), safe_id(&msg.chat_id));
                                        self.last_denial = Some(Instant::now());
                                    }
                                    continue;
                                }
                                let key = (msg.chat_id.clone(),msg.message_id.clone());
                                if self.seen.contains(&key) || self.requests.contains(&request)
                                    || heartbeat.as_ref().is_some_and(|(id,_)| id == &request)
                                    || pending.as_ref().is_some_and(|p| p.request == request) { continue; }
                                // Never wait for the model/dispatcher while holding the WS reader.
                                if self.inbound.is_closed() { break End::Cancelled; }
                                if self.inbound.try_send(InboundEvent::Message(msg)).is_err() {
                                    self.status(true, Some("WeCom inbound queue full/closed; this message was not admitted")); continue;
                                }
                                self.seen.put(key.clone(),Instant::now()); self.requests.put(request.clone(),Instant::now());
                                if anchors.len() >= 512 {
                                    if let Some(oldest) = anchors.iter().min_by_key(|(_, a)| a.created).map(|(key,_)| key.clone()) { anchors.remove(&oldest); }
                                }
                                anchors.insert(key, Anchor {request,created:Instant::now()});
                                self.status(true,None);
                            }
                        }
                        Some(Ok(Message::Ping(data))) => {
                            let result = tokio::select! { _ = self.cancel.cancelled() => false,
                                result = tokio::time::timeout(timing.ack, socket.send(Message::Pong(data))) => matches!(result,Ok(Ok(()))) };
                            if !result { break End::Network; }
                        }
                        Some(Ok(Message::Pong(_))) => {}
                        _ => break End::Network,
                    }
                }
                command = self.commands.recv(), if pending.is_none() => {
                    let Some(command) = command else { break End::Cancelled; };
                    if command.result.is_closed() { continue; }
                    let key = (command.chat,command.message);
                    let Some(anchor) = anchors.remove(&key).filter(|a| a.created.elapsed() < timing.anchor) else {
                        let _ = command.result.send(Err(error(ReplyStreamErrorKind::Expired))); continue;
                    };
                    let id = uuid::Uuid::new_v4().to_string();
                    // Claim the exact callback before writing. Rejection, timeout,
                    // cancellation and disconnect never make it reusable.
                    pending = Some(Pending { request: anchor.request.clone(), id: id.clone(), deadline: Instant::now()+timing.ack, result: command.result });
                    let frame = json!({"cmd":"aibot_respond_msg","headers":{"req_id":anchor.request},
                        "body":{"msgtype":"stream","stream":{"id":id,"finish":true,"content":command.text}}});
                    if !write(socket,frame,&self.cancel,timing.ack).await { break End::Network; }
                }
            }
        };
        if let Some(pending) = pending {
            let _ = pending
                .result
                .send(Err(error(ReplyStreamErrorKind::Ambiguous)));
        }
        end
    }
}

async fn write(
    socket: &mut Socket,
    frame: Value,
    cancel: &CancellationToken,
    deadline: Duration,
) -> bool {
    tokio::select! { _ = cancel.cancelled() => false,
    result = tokio::time::timeout(deadline,socket.send(Message::Text(frame.to_string().into()))) => matches!(result,Ok(Ok(()))) }
}

pub(super) fn callback(
    account: &ChannelAccountConfig,
    frame: &Value,
) -> Option<(MsgContext, String)> {
    if frame.get("cmd")?.as_str()? != "aibot_msg_callback" {
        return None;
    }
    let request = frame.pointer("/headers/req_id")?.as_str()?.to_owned();
    let body = frame.get("body")?;
    let bot = account.credentials.get("botId")?.as_str()?;
    if body.get("aibotid")?.as_str()? != bot || body.get("msgtype")?.as_str()? != "text" {
        return None;
    }
    let sender = body.pointer("/from/userid")?.as_str()?;
    let message = body.get("msgid")?.as_str()?;
    let text = body.pointer("/text/content")?.as_str()?;
    if !valid_id(&request)
        || !valid_id(sender)
        || !valid_id(message)
        || sender == bot
        || text.trim().is_empty()
        || text.len() > MAX_TEXT
    {
        return None;
    }
    let (chat_type, chat_id) = match body.get("chattype")?.as_str()? {
        "single" => (ChatType::Dm, format!("single:{sender}")),
        "group" => {
            let id = body.get("chatid")?.as_str()?;
            if !valid_id(id) {
                return None;
            }
            (ChatType::Group, format!("group:{id}"))
        }
        _ => return None,
    };
    Some((
        MsgContext {
            channel_id: ChannelId::WeCom,
            account_id: account.id.clone(),
            sender_id: sender.into(),
            sender_name: None,
            sender_username: None,
            sender_tenant_id: None,
            chat_id,
            chat_type,
            chat_title: None,
            thread_id: None,
            message_id: message.into(),
            text: Some(text.into()),
            media: vec![],
            reply_to_message_id: None,
            timestamp: chrono::Utc::now(),
            was_mentioned: false,
            raw: Value::Null,
        },
        request,
    ))
}
