//! IM channel integration for the `ask_user_question` tool.
//!
//! Listens for `ask_user_request` EventBus events and routes them to the IM
//! channel the owning session belongs to. Mirrors the structure of
//! [`super::approval`]: button-capable channels get native inline buttons,
//! channels without button support fall back to a numbered text prompt that
//! users answer with replies like `1a`, `2b`, or `done` (for multi-select).

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;

use ha_core::ask_user::{
    self as ask_user_mod, AskUserFileAnswer, AskUserFileConstraints, AskUserQuestionAnswer,
    AskUserQuestionGroup, AskUserTimedOutPayload,
};
use ha_core::channel::db::{ChannelConversation, ChannelDB};
use ha_core::channel::registry::ChannelRegistry;
use ha_core::channel::types::{ChannelId, ChatType, InlineButton, ReplyPayload};

use super::dispatcher::send_text_chunks;
use super::pipeline::DeliveryTarget;

/// Callback data prefix for ask_user buttons across all channels.
pub(crate) const ASK_USER_PREFIX: &str = "ask_user:";

/// Telegram's Bot API caps `callback_data` at 64 UTF-8 bytes. Keeping the
/// shared ask_user protocol within that strictest limit makes the same button
/// payload portable across every adapter.
const ASK_USER_CALLBACK_MAX_BYTES: usize = 64;

#[derive(Debug, Clone)]
pub struct InteractiveCallbackSource {
    pub channel_id: ChannelId,
    pub account_id: String,
    pub chat_id: String,
    pub thread_id: Option<String>,
}

impl InteractiveCallbackSource {
    pub fn new(
        channel_id: ChannelId,
        account_id: impl Into<String>,
        chat_id: impl Into<String>,
        thread_id: Option<&str>,
    ) -> Self {
        Self {
            channel_id,
            account_id: account_id.into(),
            chat_id: chat_id.into(),
            thread_id: thread_id
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        }
    }
}

fn normalized_thread(thread_id: Option<&str>) -> Option<&str> {
    thread_id.map(str::trim).filter(|s| !s.is_empty())
}

/// Identity of the exact `channel_conversations` attach that received an
/// interactive prompt. Route fields alone are insufficient because the same
/// chat can detach and later reattach to the same session; the row id makes
/// that new attach a distinct authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InteractiveAttachIdentity {
    session_id: String,
    attach_id: i64,
    channel_id: String,
    account_id: String,
    chat_id: String,
    thread_id: Option<String>,
}

impl InteractiveAttachIdentity {
    pub(crate) fn from_conversation(
        session_id: &str,
        conversation: &ChannelConversation,
    ) -> anyhow::Result<Self> {
        if conversation.session_id != session_id {
            return Err(anyhow::anyhow!(
                "Interactive attach session mismatch: requested {}, row belongs to {}",
                session_id,
                conversation.session_id
            ));
        }
        Ok(Self {
            session_id: session_id.to_string(),
            attach_id: conversation.id,
            channel_id: conversation.channel_id.clone(),
            account_id: conversation.account_id.clone(),
            chat_id: conversation.chat_id.clone(),
            thread_id: normalized_thread(conversation.thread_id.as_deref()).map(str::to_string),
        })
    }

    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
    }

    pub(crate) fn matches_chat(&self, account_id: &str, chat_id: &str) -> bool {
        self.account_id == account_id && self.chat_id == chat_id
    }

    fn matches_conversation(&self, conversation: &ChannelConversation) -> bool {
        self.attach_id == conversation.id
            && self.session_id == conversation.session_id
            && self.channel_id == conversation.channel_id
            && self.account_id == conversation.account_id
            && self.chat_id == conversation.chat_id
            && self.thread_id.as_deref() == normalized_thread(conversation.thread_id.as_deref())
    }

    fn validate_source(
        &self,
        callback_source: Option<&InteractiveCallbackSource>,
        source: &'static str,
    ) -> anyhow::Result<()> {
        let callback_source = callback_source.ok_or_else(|| {
            anyhow::anyhow!(
                "Interactive callback from {source} is missing source context for session {}",
                self.session_id
            )
        })?;
        let source_channel_id = callback_source.channel_id.to_string();
        let source_thread = normalized_thread(callback_source.thread_id.as_deref());
        if self.channel_id != source_channel_id
            || self.account_id != callback_source.account_id
            || self.chat_id != callback_source.chat_id
            || self.thread_id.as_deref() != source_thread
        {
            return Err(anyhow::anyhow!(
                "Interactive callback source mismatch from {} for attach {}: expected {}:{}:{}:{:?}, got {}:{}:{}:{:?}",
                source,
                self.attach_id,
                self.channel_id,
                self.account_id,
                self.chat_id,
                self.thread_id,
                source_channel_id,
                callback_source.account_id,
                callback_source.chat_id,
                source_thread,
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_live_with_db(
        &self,
        channel_db: &ChannelDB,
        source: &'static str,
    ) -> anyhow::Result<()> {
        let Some(conversation) = channel_db.get_conversation_by_session(&self.session_id)? else {
            return Err(anyhow::anyhow!(
                "No channel conversation attached to session {} while validating {}",
                self.session_id,
                source
            ));
        };
        if !self.matches_conversation(&conversation) {
            return Err(anyhow::anyhow!(
                "Interactive attach {} is no longer live for session {} while validating {}",
                self.attach_id,
                self.session_id,
                source
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_source_live(
        &self,
        callback_source: Option<&InteractiveCallbackSource>,
        source: &'static str,
    ) -> anyhow::Result<()> {
        self.validate_source(callback_source, source)?;
        let channel_db = ha_core::globals::get_channel_db().ok_or_else(|| {
            anyhow::anyhow!("ChannelDB not initialized for IM callback validation")
        })?;
        self.validate_live_with_db(&channel_db, source)
    }
}

// ── Pending state for in-progress IM answers ─────────────────────

/// One question's in-progress answer accumulator (button channels only need
/// selected values; multi-select and text fallbacks use the same state).
#[derive(Debug, Clone, Default)]
struct QuestionProgress {
    selected: Vec<String>,
    custom_input: Option<String>,
    files: Vec<AskUserFileAnswer>,
}

#[derive(Debug, Clone)]
struct PendingAskUser {
    request_id: String,
    group: AskUserQuestionGroup,
    attach_identity: InteractiveAttachIdentity,
    progress: HashMap<String, QuestionProgress>,
}

impl PendingAskUser {
    fn new(group: AskUserQuestionGroup, attach_identity: InteractiveAttachIdentity) -> Self {
        let mut progress = HashMap::new();
        for q in &group.questions {
            progress.insert(q.question_id.clone(), QuestionProgress::default());
        }
        Self {
            request_id: group.request_id.clone(),
            group,
            attach_identity,
            progress,
        }
    }

    fn into_answers(self) -> Vec<AskUserQuestionAnswer> {
        self.group
            .questions
            .iter()
            .map(|q| {
                let prog = self
                    .progress
                    .get(&q.question_id)
                    .cloned()
                    .unwrap_or_default();
                AskUserQuestionAnswer {
                    question_id: q.question_id.clone(),
                    selected: prog.selected,
                    custom_input: prog.custom_input,
                    files: prog.files,
                }
            })
            .collect()
    }

    fn is_complete(&self) -> bool {
        self.group.questions.iter().all(|q| {
            let prog = self
                .progress
                .get(&q.question_id)
                .cloned()
                .unwrap_or_default();
            !prog.selected.is_empty() || prog.custom_input.is_some() || !prog.files.is_empty()
        })
    }
}

/// Exact IM route that is allowed to answer a pending question through an
/// ordinary text message. Including channel and thread prevents a prompt in
/// one provider/topic from consuming an unrelated chat turn that happens to
/// reuse the same account/chat identifiers.
pub(crate) type InteractiveRouteKey = (String, String, String, Option<String>);

pub(crate) fn interactive_route_key(
    channel_id: &ChannelId,
    account_id: &str,
    chat_id: &str,
    thread_id: Option<&str>,
) -> InteractiveRouteKey {
    (
        channel_id.to_string(),
        account_id.to_string(),
        chat_id.to_string(),
        normalized_thread(thread_id).map(str::to_string),
    )
}

/// Canonical pending state. Button callbacks and ordinary IM replies resolve
/// through separate indices, but mutate the same `PendingAskUser` value under
/// one lock. This makes completion/cancellation remove both entry points
/// atomically and avoids cross-map lock ordering entirely.
#[derive(Default)]
struct PendingAskUserState {
    by_request: HashMap<String, PendingAskUser>,
    by_route: HashMap<InteractiveRouteKey, Vec<String>>,
}

impl PendingAskUserState {
    fn insert(
        &mut self,
        route: InteractiveRouteKey,
        pending: PendingAskUser,
        now: u64,
    ) -> Result<(), String> {
        let request_id = pending.request_id.clone();
        self.prune_route(&route, now);
        if pending_has_file_request(&pending) {
            let conflicting_request = self.by_route.get(&route).and_then(|request_ids| {
                request_ids.iter().find_map(|candidate| {
                    (candidate != &request_id
                        && self
                            .by_request
                            .get(candidate)
                            .is_some_and(pending_has_file_request))
                    .then(|| candidate.clone())
                })
            });
            if let Some(conflicting_request) = conflicting_request {
                return Err(conflicting_request);
            }
        }
        self.remove(&request_id);
        self.by_request.insert(request_id.clone(), pending);
        let route_requests = self.by_route.entry(route).or_default();
        route_requests.retain(|candidate| candidate != &request_id);
        route_requests.push(request_id);
        Ok(())
    }

    fn remove(&mut self, request_id: &str) -> Option<PendingAskUser> {
        let pending = self.by_request.remove(request_id);
        self.by_route.retain(|_, request_ids| {
            request_ids.retain(|candidate| candidate != request_id);
            !request_ids.is_empty()
        });
        pending
    }

    fn remove_for_session(&mut self, session_id: &str) {
        let request_ids = self
            .by_request
            .iter()
            .filter(|(_, pending)| pending.group.session_id == session_id)
            .map(|(request_id, _)| request_id.clone())
            .collect::<Vec<_>>();
        for request_id in request_ids {
            self.remove(&request_id);
        }
    }

    fn is_expired(&self, request_id: &str, now: u64) -> bool {
        self.by_request
            .get(request_id)
            .and_then(|pending| pending.group.timeout_at)
            .is_some_and(|timeout_at| timeout_at > 0 && now >= timeout_at)
    }

    fn prune_route(&mut self, route: &InteractiveRouteKey, now: u64) {
        let Some(request_ids) = self.by_route.get(route).cloned() else {
            return;
        };
        for request_id in request_ids {
            if !self.by_request.contains_key(&request_id) || self.is_expired(&request_id, now) {
                self.remove(&request_id);
            }
        }
    }

    /// Return the most recently registered live request for one exact route.
    /// Expired/missing ids are removed from both indices before lookup.
    fn latest_for_route(&mut self, route: &InteractiveRouteKey, now: u64) -> Option<String> {
        self.prune_route(route, now);
        self.by_route
            .get(route)
            .and_then(|request_ids| request_ids.last())
            .cloned()
    }
}

fn pending_has_file_request(pending: &PendingAskUser) -> bool {
    pending
        .group
        .questions
        .iter()
        .any(|question| pending_file_constraints(question).is_some())
}

fn validate_pending_attach_for_source(
    state: &PendingAskUserState,
    request_id: &str,
    expected_identity: &InteractiveAttachIdentity,
    callback_source: Option<&InteractiveCallbackSource>,
    source: &'static str,
) -> anyhow::Result<()> {
    let pending = state
        .by_request
        .get(request_id)
        .ok_or_else(|| anyhow::anyhow!("No pending ask_user with id {request_id}"))?;
    if pending.attach_identity != *expected_identity {
        return Err(anyhow::anyhow!(
            "ask_user attach identity changed for request {request_id}"
        ));
    }
    expected_identity.validate_source_live(callback_source, source)
}

static ASK_USER_PENDING: OnceLock<Mutex<PendingAskUserState>> = OnceLock::new();

fn get_pending_state() -> &'static Mutex<PendingAskUserState> {
    ASK_USER_PENDING.get_or_init(|| Mutex::new(PendingAskUserState::default()))
}

/// Current UNIX seconds, for comparing against `AskUserQuestionGroup.timeout_at`.
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Drop the pending entry if its `timeout_at` has already elapsed. Returns
/// `true` when the entry was expired (and removed), so callers can fail fast
/// instead of mutating a dead group.
async fn drop_if_expired(request_id: &str) -> bool {
    let now = now_secs();
    let mut state = get_pending_state().lock().await;
    let expired = state.is_expired(request_id, now);
    if expired {
        state.remove(request_id);
    }
    expired
}

/// Remove any in-memory pending state for the given request_id from both the
/// button and text-reply maps. Called by the tool execution path when a
/// question group is cancelled, timed out, or answered through a non-IM
/// channel, so stale entries don't accumulate.
pub async fn drop_pending_by_request_id(request_id: &str) {
    get_pending_state().lock().await.remove(request_id);
}

/// Remove every pending ask_user entry owned by a deleted/purged session.
pub async fn drop_pending_for_session(session_id: &str) {
    get_pending_state()
        .lock()
        .await
        .remove_for_session(session_id);
}

/// Remove only ask_user prompts whose exact attach has been evicted. The
/// eviction event can be observed after a replacement chat has already
/// registered its own prompt, so clearing the whole session would race and
/// delete valid replacement state.
pub async fn drop_stale_pending_for_session(session_id: &str) {
    let candidates = {
        let state = get_pending_state().lock().await;
        state
            .by_request
            .iter()
            .filter(|(_, pending)| pending.attach_identity.session_id() == session_id)
            .map(|(request_id, pending)| (request_id.clone(), pending.attach_identity.clone()))
            .collect::<Vec<_>>()
    };
    if candidates.is_empty() {
        return;
    }

    let Some(channel_db) = ha_core::globals::get_channel_db() else {
        // The watcher normally runs only after ChannelDB initialization. If
        // that invariant is broken, fail closed rather than retain a stale
        // control-message consumer.
        drop_pending_for_session(session_id).await;
        return;
    };
    let stale = candidates
        .into_iter()
        .filter(|(_, identity)| {
            identity
                .validate_live_with_db(&channel_db, "ask_user_eviction")
                .is_err()
        })
        .collect::<Vec<_>>();
    if stale.is_empty() {
        return;
    }

    let mut state = get_pending_state().lock().await;
    for (request_id, identity) in stale {
        let still_same_attach = state
            .by_request
            .get(&request_id)
            .is_some_and(|pending| pending.attach_identity == identity);
        if still_same_attach {
            state.remove(&request_id);
        }
    }
}

// ── Button / prompt rendering ─────────────────────────────────────

fn tr(locale: &str, row: [&'static str; 12]) -> &'static str {
    ha_core::i18n::pick_locale(locale, row)
}

#[cfg(not(test))]
fn current_locale() -> &'static str {
    ha_core::i18n::current_ui_locale()
}

#[cfg(test)]
fn current_locale() -> &'static str {
    ha_core::i18n::DEFAULT_LOCALE
}

/// Render the prompt text for a group. Includes context and all questions with
/// their options numbered so the user can reference them either via button or
/// text reply. Each field is individually truncated; the complete prompt goes
/// through the common chunker so later questions/options are never clipped to
/// one provider's single-message limit.
fn format_prompt(group: &AskUserQuestionGroup) -> String {
    format_prompt_for_locale(group, current_locale())
}

fn format_prompt_for_locale(group: &AskUserQuestionGroup, locale: &str) -> String {
    let mut out = String::new();
    out.push_str(question_from_ai_title(locale));
    out.push('\n');
    if let Some(ctx) = &group.context {
        out.push('\n');
        out.push_str(ha_core::truncate_utf8(ctx.fallback_text(), 500));
        out.push('\n');
    }
    for (qi, q) in group.questions.iter().enumerate() {
        let qtext = ha_core::truncate_utf8(q.text.fallback_text(), 500);
        out.push_str(&format!("\n{}. {}", qi + 1, qtext));
        if q.multi_select {
            out.push_str(multi_select_suffix(locale));
        }
        if let Some(constraints) = &q.file_constraints {
            out.push_str(&format!(
                "  ({}, max {} MiB)",
                allowed_file_types_display(constraints),
                constraints.max_bytes / (1024 * 1024)
            ));
        }
        out.push('\n');
        for (oi, opt) in q.options.iter().enumerate() {
            let marker = option_marker(qi, oi);
            let rec = if opt.recommended { " ★" } else { "" };
            let label = ha_core::truncate_utf8(opt.label.fallback_text(), 100);
            out.push_str(&format!("  {marker}. {label}{rec}\n"));
            if let Some(desc) = &opt.description {
                let desc = ha_core::truncate_utf8(desc.fallback_text(), 200);
                out.push_str(&format!("     {desc}\n"));
            }
        }
    }
    out
}

/// Build a marker like "1a" / "2b" for question `qi` option `oi`.
fn option_marker(qi: usize, oi: usize) -> String {
    let letter = (b'a' + oi as u8) as char;
    format!("{}{}", qi + 1, letter)
}

/// Extra text-reply hint sent alongside both button and text-only prompts.
fn text_reply_hint(group: &AskUserQuestionGroup) -> String {
    text_reply_hint_for_locale(group, current_locale())
}

fn text_reply_hint_for_locale(group: &AskUserQuestionGroup, locale: &str) -> String {
    if let Some(constraints) = group.questions.iter().find_map(pending_file_constraints) {
        let template = tr(
            locale,
            [
                "\n请在此聊天中发送下一个 {types} 文件。文件会绑定到本次请求；文字消息不会作为文件回答。",
                "\n請在此聊天中傳送下一個 {types} 檔案。檔案會綁定到本次請求；文字訊息不會作為檔案回答。",
                "\nSend the next {types} file in this chat. It will be bound to this request; a text message will not answer the file request.",
                "\nこのチャットで次の {types} ファイルを送信してください。ファイルはこのリクエストに紐づき、テキストメッセージはファイル回答として扱われません。",
                "\n이 채팅에서 다음 {types} 파일을 보내세요. 파일은 이 요청에 바인딩되며 텍스트 메시지는 파일 응답으로 처리되지 않습니다.",
                "\nEnvía el siguiente archivo {types} en este chat. Se vinculará a esta solicitud; un mensaje de texto no responderá a la solicitud de archivo.",
                "\nEnvie o próximo arquivo {types} neste chat. Ele será vinculado a esta solicitação; uma mensagem de texto não responderá à solicitação de arquivo.",
                "\nОтправьте следующий файл {types} в этом чате. Он будет привязан к этому запросу; текстовое сообщение не считается ответом с файлом.",
                "\nأرسل ملف {types} التالي في هذه الدردشة. سيتم ربطه بهذا الطلب؛ ولن تُعد الرسالة النصية إجابة لطلب الملف.",
                "\nBu sohbette sıradaki {types} dosyasını gönderin. Dosya bu isteğe bağlanır; metin mesajı dosya isteğini yanıtlamaz.",
                "\nGửi tệp {types} tiếp theo trong cuộc trò chuyện này. Tệp sẽ được liên kết với yêu cầu này; tin nhắn văn bản không được xem là câu trả lời tệp.",
                "\nHantar fail {types} seterusnya dalam sembang ini. Fail akan diikat pada permintaan ini; mesej teks tidak menjawab permintaan fail.",
            ],
        );
        return template.replace("{types}", &allowed_file_types_display(&constraints));
    }
    let has_multi = group.questions.iter().any(|q| q.multi_select);
    if has_multi {
        tr(
            locale,
            [
                "\n请用 `1a`（单选）或 `1a,1c`（多选）这样的选项标记回复，也可直接输入自由文本作为 Other 回答。完成后输入 `done`。",
                "\n請用 `1a`（單選）或 `1a,1c`（多選）這樣的選項標記回覆，也可直接輸入自由文字作為 Other 回答。完成後輸入 `done`。",
                "\nReply with option markers like `1a` (single-select) or `1a,1c` (multi-select), or type free text as an Other answer. Type `done` when finished.",
                "\n`1a`（単一選択）や `1a,1c`（複数選択）のような選択肢マーカーで返信するか、Other の回答として自由テキストを入力してください。完了したら `done` と入力してください。",
                "\n`1a`(단일 선택) 또는 `1a,1c`(다중 선택) 같은 옵션 표시로 답장하거나 Other 답변으로 자유 텍스트를 입력하세요. 완료되면 `done`을 입력하세요.",
                "\nResponde con marcadores de opción como `1a` (selección única) o `1a,1c` (selección múltiple), o escribe texto libre como respuesta Other. Escribe `done` al terminar.",
                "\nResponda com marcadores de opção como `1a` (seleção única) ou `1a,1c` (seleção múltipla), ou digite texto livre como resposta Other. Digite `done` ao terminar.",
                "\nОтветьте маркерами вариантов вроде `1a` (один выбор) или `1a,1c` (несколько вариантов), либо введите свободный текст как ответ Other. Введите `done`, когда закончите.",
                "\nرد بعلامات الخيارات مثل `1a` (اختيار واحد) أو `1a,1c` (اختيارات متعددة)، أو اكتب نصا حرا كإجابة Other. اكتب `done` عند الانتهاء.",
                "\n`1a` (tek seçim) veya `1a,1c` (çoklu seçim) gibi seçenek işaretleriyle yanıtlayın ya da Other yanıtı olarak serbest metin yazın. Bitirince `done` yazın.",
                "\nTrả lời bằng ký hiệu lựa chọn như `1a` (chọn một) hoặc `1a,1c` (chọn nhiều), hoặc nhập văn bản tự do làm câu trả lời Other. Nhập `done` khi hoàn tất.",
                "\nBalas dengan penanda pilihan seperti `1a` (pilihan tunggal) atau `1a,1c` (berbilang pilihan), atau taip teks bebas sebagai jawapan Other. Taip `done` apabila selesai.",
            ],
        )
        .to_string()
    } else {
        tr(
            locale,
            [
                "\n请用 `1a`、`2b` 这样的选项标记回复，或直接输入自由文本作为自定义回答。",
                "\n請用 `1a`、`2b` 這樣的選項標記回覆，或直接輸入自由文字作為自訂回答。",
                "\nReply with an option marker like `1a`, `2b`, or type free text to provide a custom answer.",
                "\n`1a`、`2b` のような選択肢マーカーで返信するか、自由入力でカスタム回答を送ってください。",
                "\n`1a`, `2b` 같은 옵션 표시로 답장하거나 자유 텍스트로 사용자 지정 답변을 입력하세요.",
                "\nResponde con un marcador de opción como `1a`, `2b`, o escribe texto libre para dar una respuesta personalizada.",
                "\nResponda com um marcador de opção como `1a`, `2b`, ou digite texto livre para fornecer uma resposta personalizada.",
                "\nОтветьте маркером варианта вроде `1a`, `2b`, или введите свободный текст для собственного ответа.",
                "\nرد بعلامة خيار مثل `1a` أو `2b`، أو اكتب نصا حرا لتقديم إجابة مخصصة.",
                "\n`1a`, `2b` gibi bir seçenek işaretiyle yanıtlayın veya özel yanıt için serbest metin yazın.",
                "\nTrả lời bằng ký hiệu lựa chọn như `1a`, `2b`, hoặc nhập văn bản tự do để đưa câu trả lời tùy chỉnh.",
                "\nBalas dengan penanda pilihan seperti `1a`, `2b`, atau taip teks bebas untuk memberikan jawapan tersuai.",
            ],
        )
        .to_string()
    }
}

fn allowed_file_types_display(constraints: &AskUserFileConstraints) -> String {
    let labels = [
        ("application/pdf", "PDF"),
        ("text/plain", "TXT"),
        ("text/markdown", "MD"),
    ]
    .into_iter()
    .filter_map(|(mime_type, label)| {
        constraints
            .types
            .iter()
            .any(|candidate| candidate == mime_type)
            .then_some(label)
    })
    .collect::<Vec<_>>();
    if labels.is_empty() {
        "no supported types".to_string()
    } else {
        labels.join("/")
    }
}

/// Build inline button rows for button-capable channels.
///
/// Callback payloads carry only stable question/option indices. Current
/// request ids are at most 36 bytes (`auq_` + simple UUID), so even the longest
/// select payload stays below Telegram's 64-byte callback-data ceiling. The
/// provider preflight remains a second boundary if a future producer changes
/// that request-id contract.
///
/// Each question's options form one row; multi-select questions get a
/// trailing "Done" button row.
fn build_buttons(group: &AskUserQuestionGroup) -> Vec<Vec<InlineButton>> {
    build_buttons_for_locale(group, current_locale())
}

fn build_buttons_with_discord_file_upload(group: &AskUserQuestionGroup) -> Vec<Vec<InlineButton>> {
    let mut rows = build_buttons_for_locale(group, current_locale());
    if let Some(question_index) = group
        .questions
        .iter()
        .position(|question| question.input_kind.as_deref() == Some("file"))
    {
        let upload = InlineButton {
            text: "Upload file".to_string(),
            callback_data: Some(format!(
                "{}{}:f:{question_index}",
                ASK_USER_PREFIX, group.request_id
            )),
            url: None,
        };
        // Keep Cancel last and make the primary action the first row.
        rows.insert(0, vec![upload]);
    }
    rows
}

fn build_buttons_for_locale(group: &AskUserQuestionGroup, locale: &str) -> Vec<Vec<InlineButton>> {
    let mut rows: Vec<Vec<InlineButton>> = Vec::new();
    for (qi, q) in group.questions.iter().enumerate() {
        let mut row = Vec::new();
        for (oi, opt) in q.options.iter().enumerate() {
            let marker = option_marker(qi, oi);
            let text = if opt.recommended {
                format!("★ {}", opt.label.fallback_text())
            } else {
                opt.label.fallback_text().to_string()
            };
            row.push(InlineButton {
                text: format!("[{marker}] {text}"),
                callback_data: Some(format!(
                    "{}{}:s:{qi}:{oi}",
                    ASK_USER_PREFIX, group.request_id
                )),
                url: None,
            });
            // Split into chunks of 3 to keep Telegram rows short.
            if row.len() == 3 {
                rows.push(std::mem::take(&mut row));
            }
        }
        if !row.is_empty() {
            rows.push(std::mem::take(&mut row));
        }
        if q.multi_select {
            rows.push(vec![InlineButton {
                text: done_button_text(locale, qi + 1),
                callback_data: Some(format!("{}{}:d:{qi}", ASK_USER_PREFIX, group.request_id)),
                url: None,
            }]);
        }
    }
    // Top-level cancel
    rows.push(vec![InlineButton {
        text: cancel_button_text(locale).to_string(),
        callback_data: Some(format!("{}{}:c", ASK_USER_PREFIX, group.request_id)),
        url: None,
    }]);
    rows
}

fn buttons_fit_callback_limit(buttons: &[Vec<InlineButton>]) -> bool {
    buttons.iter().flatten().all(|button| {
        button.callback_data.as_deref().is_some_and(|callback| {
            !callback.is_empty() && callback.len() <= ASK_USER_CALLBACK_MAX_BYTES
        })
    })
}

fn question_from_ai_title(locale: &str) -> &'static str {
    tr(
        locale,
        [
            "❓ AI 的问题",
            "❓ AI 的問題",
            "❓ Question from AI",
            "❓ AI からの質問",
            "❓ AI의 질문",
            "❓ Pregunta de la IA",
            "❓ Pergunta da IA",
            "❓ Вопрос от ИИ",
            "❓ سؤال من الذكاء الاصطناعي",
            "❓ AI'dan soru",
            "❓ Câu hỏi từ AI",
            "❓ Soalan daripada AI",
        ],
    )
}

fn multi_select_suffix(locale: &str) -> &'static str {
    tr(
        locale,
        [
            "  （可多选）",
            "  （可複選）",
            "  (multi-select)",
            "  （複数選択）",
            "  (다중 선택)",
            "  (selección múltiple)",
            "  (seleção múltipla)",
            "  (множественный выбор)",
            "  (اختيارات متعددة)",
            "  (çoklu seçim)",
            "  (chọn nhiều)",
            "  (berbilang pilihan)",
        ],
    )
}

fn done_button_text(locale: &str, question_number: usize) -> String {
    let template = tr(
        locale,
        [
            "✅ 完成问题 {n}",
            "✅ 完成問題 {n}",
            "✅ Done with Q{n}",
            "✅ 質問 {n} を完了",
            "✅ 질문 {n} 완료",
            "✅ Terminar P{n}",
            "✅ Concluir P{n}",
            "✅ Готово с вопросом {n}",
            "✅ انتهى السؤال {n}",
            "✅ Soru {n} tamam",
            "✅ Xong câu {n}",
            "✅ Selesai S{n}",
        ],
    );
    template.replace("{n}", &question_number.to_string())
}

fn cancel_button_text(locale: &str) -> &'static str {
    tr(
        locale,
        [
            "❌ 取消",
            "❌ 取消",
            "❌ Cancel",
            "❌ キャンセル",
            "❌ 취소",
            "❌ Cancelar",
            "❌ Cancelar",
            "❌ Отмена",
            "❌ إلغاء",
            "❌ İptal",
            "❌ Hủy",
            "❌ Batal",
        ],
    )
}

// ── EventBus listener ─────────────────────────────────────────────

/// Spawn a background task that forwards `ask_user_request` events to
/// whichever IM channel the owning session belongs to. Idempotent — callers
/// should only invoke once at startup.
pub fn spawn_channel_ask_user_listener(channel_db: Arc<ChannelDB>, registry: Arc<ChannelRegistry>) {
    let Some(bus) = ha_core::globals::get_event_bus() else {
        return;
    };
    let mut rx = bus.subscribe();

    tokio::spawn(async move {
        loop {
            let event = match rx.recv().await {
                Ok(ev) => ev,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    app_warn!(
                        "channel",
                        "ask_user",
                        "ask_user listener lagged {} events",
                        n
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };

            if event.name == ask_user_mod::EVENT_ASK_USER_RESOLVED {
                if let Some(request_id) = event
                    .payload
                    .get("requestId")
                    .and_then(serde_json::Value::as_str)
                {
                    drop_pending_by_request_id(request_id).await;
                }
                continue;
            }
            if event.name == ask_user_mod::EVENT_ASK_USER_TIMED_OUT {
                handle_timeout_event(event.payload.clone(), channel_db.clone(), registry.clone())
                    .await;
                continue;
            }
            if event.name != ask_user_mod::EVENT_ASK_USER_REQUEST {
                continue;
            }

            let group: AskUserQuestionGroup = match serde_json::from_value(event.payload.clone()) {
                Ok(g) => g,
                Err(e) => {
                    app_warn!(
                        "channel",
                        "ask_user",
                        "Failed to parse ask_user group: {}",
                        e
                    );
                    continue;
                }
            };

            // Look up which channel conversation this session belongs to.
            let conversation = match channel_db.get_conversation_by_session(&group.session_id) {
                Ok(Some(conv)) => conv,
                Ok(None) => continue, // Not an IM session
                Err(e) => {
                    app_warn!(
                        "channel",
                        "ask_user",
                        "Failed to look up channel session {}: {}",
                        group.session_id,
                        e
                    );
                    continue;
                }
            };
            let attach_identity = match InteractiveAttachIdentity::from_conversation(
                &group.session_id,
                &conversation,
            ) {
                Ok(identity) => identity,
                Err(error) => {
                    app_warn!(
                        "channel",
                        "ask_user",
                        "Invalid ask_user attach identity for {}: {}",
                        group.request_id,
                        error
                    );
                    continue;
                }
            };

            let store = ha_core::config::cached_config();
            let account_config = match store.channels.find_account(&conversation.account_id) {
                Some(c) => c.clone(),
                None => continue,
            };

            let channel_id: ha_core::channel::types::ChannelId = match serde_json::from_value(
                serde_json::Value::String(conversation.channel_id.clone()),
            ) {
                Ok(id) => id,
                Err(_) => continue,
            };

            let Some(plugin) = registry.get_plugin(&channel_id) else {
                app_warn!(
                    "channel",
                    "ask_user",
                    "No channel plugin available for ask_user prompt ({})",
                    channel_id
                );
                continue;
            };
            let supports_buttons =
                plugin.supports_reply_buttons(&conversation.account_id, &conversation.chat_id);

            // Button-capable prompts keep the same ordinary-text route as their
            // buttons. Pure text/textarea questions can therefore render a
            // Cancel button while the user's next IM message supplies `Other`.
            let discord_file_upload = channel_id == ChannelId::Discord
                && account_config.discord_file_requests_enabled()
                && group
                    .questions
                    .iter()
                    .any(|question| question.input_kind.as_deref() == Some("file"));
            let candidate_buttons = if discord_file_upload {
                build_buttons_with_discord_file_upload(&group)
            } else {
                build_buttons(&group)
            };
            let callbacks_fit = buttons_fit_callback_limit(&candidate_buttons);
            if supports_buttons && !callbacks_fit {
                app_warn!(
                    "channel",
                    "ask_user",
                    "ask_user callback data exceeds the portable {}-byte limit; using text interaction",
                    ASK_USER_CALLBACK_MAX_BYTES
                );
            }
            let use_buttons = supports_buttons
                && callbacks_fit
                && match plugin.validate_reply_buttons(&candidate_buttons) {
                    Ok(()) => true,
                    Err(error) => {
                        app_warn!(
                            "channel",
                            "ask_user",
                            "Button prompt failed provider preflight; using text interaction: {}",
                            ha_core::logging::redact_sensitive(&error.to_string())
                        );
                        false
                    }
                };

            let prompt_text = format_prompt(&group);
            let prompt_with_hint = format!("{}{}", prompt_text, text_reply_hint(&group));

            // Register one canonical object plus its exact text-reply route for
            // both rendering modes. Callback and text handlers now mutate this
            // same value and terminal removal clears both indices atomically.
            let route = interactive_route_key(
                &channel_id,
                &conversation.account_id,
                &conversation.chat_id,
                conversation.thread_id.as_deref(),
            );
            let conflicting_file_request = get_pending_state()
                .lock()
                .await
                .insert(
                    route,
                    PendingAskUser::new(group.clone(), attach_identity.clone()),
                    now_secs(),
                )
                .err();
            if let Some(conflicting_request_id) = conflicting_file_request {
                ask_user_mod::cancel_pending_ask_user_question_with_source(
                    &group.request_id,
                    "channel_file_route_conflict",
                )
                .await;
                app_warn!(
                    "channel",
                    "ask_user",
                    "Rejected concurrent file request {} on a route already owned by {}",
                    group.request_id,
                    conflicting_request_id
                );
                continue;
            }

            let payload = if use_buttons {
                ReplyPayload {
                    text: Some(prompt_with_hint),
                    buttons: candidate_buttons,
                    thread_id: conversation.thread_id.clone(),
                    ..ReplyPayload::text("")
                }
            } else {
                ReplyPayload {
                    text: Some(prompt_with_hint),
                    thread_id: conversation.thread_id.clone(),
                    ..ReplyPayload::text("")
                }
            };

            let chat_type = ChatType::from_lowercase(&conversation.chat_type);
            let target = DeliveryTarget {
                account_id: &account_config.id,
                chat_id: &conversation.chat_id,
                chat_type: &chat_type,
                thread_id: conversation.thread_id.as_deref(),
                reply_to_message_id: None,
                recipient_user_id: conversation.sender_id.as_deref(),
                recipient_tenant_id: None,
            };
            if let Err(error) =
                attach_identity.validate_live_with_db(&channel_db, "ask_user_prompt_send")
            {
                drop_pending_by_request_id(&group.request_id).await;
                app_warn!(
                    "channel",
                    "ask_user",
                    "Skipped ask_user prompt for a stale attach ({}): {}",
                    group.request_id,
                    error
                );
                continue;
            }
            // A database identity check cannot be atomic with an external
            // provider send: handover can still occur after this check and
            // before the provider accepts the request. Eviction cleanup and
            // the response-side identity guard close that residual window.
            let report = send_text_chunks(
                &plugin,
                &target,
                payload.text.as_deref().unwrap_or_default(),
                None,
                &payload.buttons,
            )
            .await;
            if !report.is_success() {
                // A partially/fully failed prompt must not consume the user's
                // next ordinary chat message through an invisible stale route.
                drop_pending_by_request_id(&group.request_id).await;
                app_warn!(
                    "channel",
                    "ask_user",
                    "Failed to send ask_user prompt to channel ({} failure(s))",
                    report.failures.len()
                );
            }
        }
    });
}

async fn handle_timeout_event(
    payload: serde_json::Value,
    channel_db: Arc<ChannelDB>,
    registry: Arc<ChannelRegistry>,
) {
    let event: AskUserTimedOutPayload = match serde_json::from_value(payload) {
        Ok(e) => e,
        Err(err) => {
            app_warn!(
                "channel",
                "ask_user",
                "Failed to parse ask_user_timed_out payload: {}",
                err
            );
            return;
        }
    };

    let conversation = match channel_db.get_conversation_by_session(&event.session_id) {
        Ok(Some(c)) => c,
        Ok(None) => return,
        Err(e) => {
            app_warn!(
                "channel",
                "ask_user",
                "Timeout lookup failed for session {}: {}",
                event.session_id,
                e
            );
            return;
        }
    };

    let store = ha_core::config::cached_config();
    let locale = ha_core::i18n::effective_ui_locale(&store);
    let account_config = match store.channels.find_account(&conversation.account_id) {
        Some(c) => c.clone(),
        None => return,
    };

    let tag = id_tag(&event.request_id);
    let question = event
        .question_preview
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|s| format!("\n\n{}", ha_core::truncate_utf8(s.trim(), 500)));
    let body = ask_user_timeout_notice(
        locale,
        &tag,
        event.timeout_secs,
        event.used_default_values,
        question.as_deref().unwrap_or_default(),
    );
    let payload = ReplyPayload {
        text: Some(body),
        thread_id: conversation.thread_id.clone(),
        ..ReplyPayload::text("")
    };
    if let Err(e) = registry
        .send_reply(&account_config, &conversation.chat_id, &payload)
        .await
    {
        app_warn!(
            "channel",
            "ask_user",
            "Failed to send ask_user-timeout notice: {}",
            e
        );
    }
}

fn ask_user_timeout_notice(
    locale: &str,
    tag: &str,
    timeout_secs: u64,
    used_default_values: bool,
    question_preview: &str,
) -> String {
    let template = if used_default_values {
        tr(
            locale,
            [
                "⏱ 问题 `#{tag}` 已在 {secs} 秒后超时。我已使用配置的默认答案继续。{question}",
                "⏱ 問題 `#{tag}` 已在 {secs} 秒後逾時。我已使用設定的預設答案繼續。{question}",
                "⏱ Question #{tag} timed out after {secs}s. I continued with the configured default answer(s).{question}",
                "⏱ 質問 `#{tag}` は {secs} 秒後にタイムアウトしました。設定済みのデフォルト回答で続行しました。{question}",
                "⏱ 질문 `#{tag}`가 {secs}초 후 시간 초과되었습니다. 구성된 기본 답변으로 계속했습니다.{question}",
                "⏱ La pregunta `#{tag}` agotó el tiempo tras {secs}s. Continué con las respuestas predeterminadas configuradas.{question}",
                "⏱ A pergunta `#{tag}` expirou após {secs}s. Continuei com as respostas padrão configuradas.{question}",
                "⏱ Вопрос `#{tag}` истек через {secs} с. Я продолжил с настроенными ответами по умолчанию.{question}",
                "⏱ انتهت مهلة السؤال `#{tag}` بعد {secs} ثانية. تابعت بالإجابات الافتراضية المكونة.{question}",
                "⏱ `#{tag}` sorusu {secs} sn sonra zaman aşımına uğradı. Yapılandırılmış varsayılan yanıtlarla devam ettim.{question}",
                "⏱ Câu hỏi `#{tag}` đã hết hạn sau {secs} giây. Tôi đã tiếp tục với câu trả lời mặc định đã cấu hình.{question}",
                "⏱ Soalan `#{tag}` tamat masa selepas {secs}s. Saya meneruskan dengan jawapan lalai yang dikonfigurasi.{question}",
            ],
        )
    } else {
        tr(
            locale,
            [
                "⏱ 问题 `#{tag}` 已在 {secs} 秒后超时，且没有收到回答。如果你仍想回复，请再问我一次。{question}",
                "⏱ 問題 `#{tag}` 已在 {secs} 秒後逾時，且沒有收到回答。如果你仍想回覆，請再問我一次。{question}",
                "⏱ Question #{tag} timed out after {secs}s without an answer. Ask me again if you still want to respond.{question}",
                "⏱ 質問 `#{tag}` は回答なしで {secs} 秒後にタイムアウトしました。まだ回答したい場合はもう一度依頼してください。{question}",
                "⏱ 질문 `#{tag}`가 답변 없이 {secs}초 후 시간 초과되었습니다. 여전히 답하고 싶다면 다시 요청해 주세요.{question}",
                "⏱ La pregunta `#{tag}` agotó el tiempo tras {secs}s sin respuesta. Vuelve a pedírmelo si aún quieres responder.{question}",
                "⏱ A pergunta `#{tag}` expirou após {secs}s sem resposta. Peça novamente se ainda quiser responder.{question}",
                "⏱ Вопрос `#{tag}` истек через {secs} с без ответа. Попросите снова, если все еще хотите ответить.{question}",
                "⏱ انتهت مهلة السؤال `#{tag}` بعد {secs} ثانية بلا إجابة. اسألني مرة أخرى إذا كنت لا تزال تريد الرد.{question}",
                "⏱ `#{tag}` sorusu yanıtsız olarak {secs} sn sonra zaman aşımına uğradı. Hâlâ yanıtlamak istiyorsanız tekrar isteyin.{question}",
                "⏱ Câu hỏi `#{tag}` đã hết hạn sau {secs} giây mà không có câu trả lời. Hãy hỏi lại nếu bạn vẫn muốn phản hồi.{question}",
                "⏱ Soalan `#{tag}` tamat masa selepas {secs}s tanpa jawapan. Minta saya lagi jika masih mahu membalas.{question}",
            ],
        )
    };
    template
        .replace("{tag}", tag)
        .replace("{secs}", &timeout_secs.to_string())
        .replace("{question}", question_preview)
}

fn id_tag(request_id: &str) -> String {
    request_id.chars().take(8).collect()
}

fn ask_user_callback_cancelled(locale: &str) -> &'static str {
    tr(
        locale,
        [
            "❌ 已取消",
            "❌ 已取消",
            "❌ Cancelled",
            "❌ キャンセルしました",
            "❌ 취소됨",
            "❌ Cancelado",
            "❌ Cancelado",
            "❌ Отменено",
            "❌ تم الإلغاء",
            "❌ İptal edildi",
            "❌ Đã hủy",
            "❌ Dibatalkan",
        ],
    )
}

fn ask_user_callback_selected(locale: &str) -> &'static str {
    tr(
        locale,
        [
            "✓ 已选择",
            "✓ 已選擇",
            "✓ Selected",
            "✓ 選択しました",
            "✓ 선택됨",
            "✓ Seleccionado",
            "✓ Selecionado",
            "✓ Выбрано",
            "✓ تم الاختيار",
            "✓ Seçildi",
            "✓ Đã chọn",
            "✓ Dipilih",
        ],
    )
}

fn ask_user_callback_answered(locale: &str) -> &'static str {
    tr(
        locale,
        [
            "✅ 已回答",
            "✅ 已回答",
            "✅ Answered",
            "✅ 回答しました",
            "✅ 답변됨",
            "✅ Respondido",
            "✅ Respondido",
            "✅ Отвечено",
            "✅ تمت الإجابة",
            "✅ Yanıtlandı",
            "✅ Đã trả lời",
            "✅ Dijawab",
        ],
    )
}

fn ask_user_callback_incomplete(locale: &str) -> &'static str {
    tr(
        locale,
        [
            "还需要回答其余问题",
            "還需要回答其餘問題",
            "Please answer the remaining questions",
            "残りの質問に回答してください",
            "남은 질문에 답해 주세요",
            "Responde las preguntas restantes",
            "Responda às perguntas restantes",
            "Ответьте на оставшиеся вопросы",
            "يرجى الإجابة عن الأسئلة المتبقية",
            "Lütfen kalan soruları yanıtlayın",
            "Vui lòng trả lời các câu hỏi còn lại",
            "Sila jawab soalan yang masih belum dijawab",
        ],
    )
}

fn ask_user_source_mismatch(locale: &str) -> &'static str {
    tr(
        locale,
        [
            "ℹ️ 这个问题现在属于另一个会话，不能从这里回答。请在当前显示问题提示的聊天里回复。",
            "ℹ️ 這個問題目前屬於另一個對話，不能從這裡回答。請在目前顯示問題提示的聊天中回覆。",
            "ℹ️ This question belongs to a different conversation now and can't be answered from here. Reply in the chat where the question prompt currently appears.",
            "ℹ️ この質問は現在別の会話に属しているため、ここからは回答できません。質問プロンプトが表示されているチャットで返信してください。",
            "ℹ️ 이 질문은 이제 다른 대화에 속해 있어 여기서 답할 수 없습니다. 질문 프롬프트가 표시된 채팅에서 답해 주세요.",
            "ℹ️ Esta pregunta ahora pertenece a otra conversación y no puede responderse desde aquí. Responde en el chat donde aparece la pregunta.",
            "ℹ️ Esta pergunta agora pertence a outra conversa e não pode ser respondida daqui. Responda no chat onde a pergunta aparece.",
            "ℹ️ Этот вопрос теперь относится к другому разговору, и здесь на него нельзя ответить. Ответьте в чате, где показан вопрос.",
            "ℹ️ هذا السؤال ينتمي الآن إلى محادثة أخرى ولا يمكن الرد عليه من هنا. أجب في الدردشة التي يظهر فيها السؤال.",
            "ℹ️ Bu soru artık farklı bir konuşmaya ait ve buradan yanıtlanamaz. Sorunun göründüğü sohbette yanıtlayın.",
            "ℹ️ Câu hỏi này hiện thuộc một cuộc trò chuyện khác và không thể trả lời từ đây. Hãy trả lời trong cuộc trò chuyện đang hiển thị câu hỏi.",
            "ℹ️ Soalan ini kini milik perbualan lain dan tidak boleh dijawab dari sini. Balas dalam sembang tempat soalan dipaparkan.",
        ],
    )
}

async fn send_text_reply_feedback(msg: &ha_core::channel::types::MsgContext, text: &str) {
    let Some(registry) = ha_core::globals::get_channel_registry() else {
        app_warn!(
            "channel",
            "ask_user",
            "Cannot send ask_user text feedback before ChannelRegistry initialization"
        );
        return;
    };
    let account = {
        let store = ha_core::config::cached_config();
        store.channels.find_account(&msg.account_id).cloned()
    };
    let Some(account) = account else {
        app_warn!(
            "channel",
            "ask_user",
            "Cannot send ask_user text feedback: account {} is unavailable",
            msg.account_id
        );
        return;
    };
    let payload = ReplyPayload {
        text: Some(text.to_string()),
        thread_id: msg.thread_id.clone(),
        ..ReplyPayload::text("")
    };
    if let Err(error) = registry.send_reply(&account, &msg.chat_id, &payload).await {
        app_warn!(
            "channel",
            "ask_user",
            "Failed to send ask_user text feedback: {}",
            ha_core::logging::redact_sensitive(&error.to_string())
        );
    }
}

// ── Text-reply handler (channels without buttons) ─────────────────

fn question_accepts_custom_input(question: &ha_core::ask_user::AskUserQuestion) -> bool {
    question.input_kind.as_deref() != Some("file")
        && (question.allow_custom
            || matches!(question.input_kind.as_deref(), Some("text" | "textarea")))
}

/// Return whether the latest live prompt on this exact route is a file
/// request. The dispatcher uses this before any network-facing media
/// hydration, after access and mention gating have already passed.
pub async fn has_pending_file_request(msg: &ha_core::channel::types::MsgContext) -> bool {
    let route = interactive_route_key(
        &msg.channel_id,
        &msg.account_id,
        &msg.chat_id,
        msg.thread_id.as_deref(),
    );
    let mut state = get_pending_state().lock().await;
    let Some(request_id) = state.latest_for_route(&route, now_secs()) else {
        return false;
    };
    let Some(pending) = state.by_request.get(&request_id) else {
        return false;
    };
    pending.group.questions.len() == 1
        && pending
            .group
            .questions
            .first()
            .is_some_and(|question| pending_file_constraints(question).is_some())
}

fn pending_file_constraints(
    question: &ha_core::ask_user::AskUserQuestion,
) -> Option<AskUserFileConstraints> {
    (question.input_kind.as_deref() == Some("file"))
        .then(|| question.file_constraints.clone())
        .flatten()
}

fn validate_and_persist_file_answer(
    media: Vec<ha_core::channel::types::InboundMedia>,
    session_id: &str,
    constraints: &AskUserFileConstraints,
) -> anyhow::Result<AskUserFileAnswer> {
    if constraints.count != 1 || media.len() != 1 {
        anyhow::bail!("send exactly one file");
    }
    let item = &media[0];
    if item
        .file_size
        .is_some_and(|size| size > constraints.max_bytes)
    {
        anyhow::bail!("the declared file size exceeds the request limit");
    }
    let source = item
        .file_url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("the provider did not materialize the file"))?;
    let canonical_source = std::path::Path::new(source)
        .canonicalize()
        .map_err(|_| anyhow::anyhow!("the downloaded file is unavailable"))?;
    let channels_root = ha_core::paths::channels_dir()?.canonicalize()?;
    if !canonical_source.starts_with(&channels_root) {
        anyhow::bail!("the downloaded file is outside the channel media boundary");
    }
    let actual_size = std::fs::metadata(&canonical_source)?.len();
    if actual_size == 0 || actual_size > constraints.max_bytes {
        anyhow::bail!("the actual file size is empty or exceeds the request limit");
    }

    let extension = canonical_source
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let bytes = std::fs::read(&canonical_source)?;
    let detected_mime = if extension == "pdf" && bytes.starts_with(b"%PDF-") {
        "application/pdf"
    } else if matches!(extension.as_str(), "txt" | "md")
        && !bytes.contains(&0)
        && std::str::from_utf8(&bytes).is_ok()
    {
        if extension == "md" {
            "text/markdown"
        } else {
            "text/plain"
        }
    } else {
        anyhow::bail!("the file content is not a supported PDF, TXT, or MD document");
    };
    if !constraints.types.iter().any(|value| value == detected_mime) {
        anyhow::bail!("the detected file type is not allowed by this request");
    }

    let original_name = canonical_source
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(&item.file_id)
        .to_string();
    let mut attachments = super::media::convert_inbound_media_to_attachments(&media, session_id);
    let attachment = attachments
        .pop()
        .ok_or_else(|| anyhow::anyhow!("the file could not be persisted for this session"))?;
    let file_path = attachment
        .file_path
        .ok_or_else(|| anyhow::anyhow!("the persisted file path is unavailable"))?;
    let canonical_attachment = std::path::Path::new(&file_path).canonicalize()?;
    let attachment_root = ha_core::paths::attachments_dir(session_id)?.canonicalize()?;
    if !canonical_attachment.starts_with(&attachment_root) {
        anyhow::bail!("the persisted file escaped the session attachment boundary");
    }

    Ok(AskUserFileAnswer {
        name: original_name,
        mime_type: detected_mime.to_string(),
        size: actual_size,
        file_path: canonical_attachment.to_string_lossy().to_string(),
    })
}

/// Bind a hydrated attachment to the latest live file request on the exact IM
/// route. Invalid attachments are consumed with feedback while the request
/// remains pending, allowing the user to try again safely.
pub async fn try_handle_ask_user_file_reply(
    msg: &ha_core::channel::types::MsgContext,
    session_id: &str,
) -> bool {
    let route = interactive_route_key(
        &msg.channel_id,
        &msg.account_id,
        &msg.chat_id,
        msg.thread_id.as_deref(),
    );
    let (request_id, question_id, constraints, attach_identity) = {
        let mut state = get_pending_state().lock().await;
        let Some(request_id) = state.latest_for_route(&route, now_secs()) else {
            return false;
        };
        let Some(pending) = state.by_request.get(&request_id) else {
            return false;
        };
        let Some(question) = pending.group.questions.first() else {
            return false;
        };
        let Some(constraints) = pending_file_constraints(question) else {
            return false;
        };
        if pending.group.questions.len() != 1 || pending.attach_identity.session_id() != session_id
        {
            return false;
        }
        (
            request_id,
            question.question_id.clone(),
            constraints,
            pending.attach_identity.clone(),
        )
    };

    if msg.media.is_empty() {
        let allowed_types = allowed_file_types_display(&constraints);
        send_text_reply_feedback(
            msg,
            &format!(
                "⚠️ The attachment could not be downloaded or was rejected by the provider. Please send one {allowed_types} file again."
            ),
        )
        .await;
        return true;
    }

    let reply_source = InteractiveCallbackSource::new(
        msg.channel_id.clone(),
        msg.account_id.clone(),
        msg.chat_id.clone(),
        msg.thread_id.as_deref(),
    );
    if let Err(error) =
        attach_identity.validate_source_live(Some(&reply_source), "file_reply_preflight")
    {
        get_pending_state().lock().await.remove(&request_id);
        app_warn!(
            "channel",
            "ask_user",
            "File ask_user source mismatch for {}: {}",
            request_id,
            ha_core::logging::redact_sensitive(&error.to_string())
        );
        send_text_reply_feedback(msg, ask_user_source_mismatch(current_locale())).await;
        return true;
    }

    let media = msg.media.clone();
    let validation_session = session_id.to_string();
    let validation_constraints = constraints.clone();
    let answer = ha_core::blocking::run_blocking(move || {
        validate_and_persist_file_answer(media, &validation_session, &validation_constraints)
    })
    .await;
    let answer = match answer {
        Ok(answer) => answer,
        Err(error) => {
            let allowed_types = allowed_file_types_display(&constraints);
            let message = format!(
                "⚠️ File not accepted: {}. Send one {allowed_types} file within the requested size limit.",
                ha_core::logging::redact_sensitive(&error.to_string())
            );
            send_text_reply_feedback(msg, &message).await;
            return true;
        }
    };

    let pending = {
        let mut state = get_pending_state().lock().await;
        if state.latest_for_route(&route, now_secs()).as_deref() != Some(request_id.as_str())
            || state
                .by_request
                .get(&request_id)
                .map_or(true, |pending| pending.attach_identity != attach_identity)
        {
            return true;
        }
        if let Some(progress) = state
            .by_request
            .get_mut(&request_id)
            .and_then(|pending| pending.progress.get_mut(&question_id))
        {
            progress.files = vec![answer];
        }
        state.remove(&request_id)
    };
    let Some(pending) = pending else {
        return true;
    };
    if let Err(error) =
        attach_identity.validate_source_live(Some(&reply_source), "file_reply_submit")
    {
        app_warn!(
            "channel",
            "ask_user",
            "Skipped ask_user file submit after attach changed ({}): {}",
            request_id,
            ha_core::logging::redact_sensitive(&error.to_string())
        );
        send_text_reply_feedback(msg, ask_user_source_mismatch(current_locale())).await;
        return true;
    }
    if let Err(error) =
        ask_user_mod::submit_ask_user_question_response(&request_id, pending.into_answers()).await
    {
        app_warn!(
            "channel",
            "ask_user",
            "Failed to submit ask_user file answer ({}): {}",
            request_id,
            error
        );
    }
    true
}

/// Apply an ordinary IM message as an Other/free-text answer. Prefer the first
/// unanswered question. If all questions already have button selections, only
/// accept one unambiguous custom target. Multi-select preserves its selected
/// values (matching the GUI), while single-select Other replaces its option.
fn apply_text_custom_input(pending: &mut PendingAskUser, text: &str) -> bool {
    let unanswered = pending
        .group
        .questions
        .iter()
        .enumerate()
        .find_map(|(index, question)| {
            if !question_accepts_custom_input(question) {
                return None;
            }
            let progress = pending
                .progress
                .get(&question.question_id)
                .cloned()
                .unwrap_or_default();
            (progress.selected.is_empty() && progress.custom_input.is_none()).then_some(index)
        });

    let question_index = unanswered.or_else(|| {
        let mut candidates =
            pending
                .group
                .questions
                .iter()
                .enumerate()
                .filter_map(|(index, question)| {
                    if !question_accepts_custom_input(question) {
                        return None;
                    }
                    let has_custom_input = pending
                        .progress
                        .get(&question.question_id)
                        .and_then(|progress| progress.custom_input.as_ref())
                        .is_some();
                    (!has_custom_input).then_some(index)
                });
        let only = candidates.next()?;
        candidates.next().is_none().then_some(only)
    });

    let Some(question_index) = question_index else {
        return false;
    };
    let question = &pending.group.questions[question_index];
    let progress = pending
        .progress
        .entry(question.question_id.clone())
        .or_default();
    if !question.multi_select {
        progress.selected.clear();
    }
    progress.custom_input = Some(text.to_string());
    true
}

/// Try to interpret an inbound IM message as an ask_user text reply on any
/// channel, including button-capable providers.
/// Returns `true` if the message was consumed.
///
/// Accepted reply formats:
/// - `1a`         single option for Q1
/// - `1a,1c`      multi-select for Q1
/// - `done`       finalise all answers (multi-select)
/// - `cancel`     abort the group
/// - `<text>`     free-form custom input for the first unanswered question
pub async fn try_handle_ask_user_reply(msg: &ha_core::channel::types::MsgContext) -> bool {
    let text = match msg.text.as_deref() {
        Some(t) => t.trim().to_string(),
        None => return false,
    };
    if text.is_empty() {
        return false;
    }

    let route = interactive_route_key(
        &msg.channel_id,
        &msg.account_id,
        &msg.chat_id,
        msg.thread_id.as_deref(),
    );
    let (request_id, attach_identity) = {
        let mut state = get_pending_state().lock().await;
        // Operate on the most recent group for this exact channel/account/chat/
        // thread route. Lookup atomically evicts expired groups first, so a late
        // text reply cannot re-animate timed-out tool state.
        let Some(request_id) = state.latest_for_route(&route, now_secs()) else {
            return false;
        };
        let Some(attach_identity) = state
            .by_request
            .get(&request_id)
            .map(|pending| pending.attach_identity.clone())
        else {
            state.remove(&request_id);
            return false;
        };
        (request_id, attach_identity)
    };

    // The session may have been handed over after this prompt was rendered.
    // Revalidate the live session→chat binding exactly like button callbacks;
    // consume mismatched control text so it cannot leak into a different turn.
    let reply_source = InteractiveCallbackSource::new(
        msg.channel_id.clone(),
        msg.account_id.clone(),
        msg.chat_id.clone(),
        msg.thread_id.as_deref(),
    );
    if let Err(error) = attach_identity.validate_source_live(Some(&reply_source), "text_reply") {
        let mut state = get_pending_state().lock().await;
        if state
            .by_request
            .get(&request_id)
            .is_some_and(|pending| pending.attach_identity == attach_identity)
        {
            state.remove(&request_id);
        }
        drop(state);
        app_warn!(
            "channel",
            "ask_user",
            "Text ask_user reply source mismatch for {}: {}",
            request_id,
            ha_core::logging::redact_sensitive(&error.to_string())
        );
        send_text_reply_feedback(msg, ask_user_source_mismatch(current_locale())).await;
        return true;
    }

    let mut state = get_pending_state().lock().await;
    // Callback validation performs synchronous DB work outside the pending
    // lock. Recheck liveness/latest ownership after reacquiring it so a timeout,
    // cancellation, or newer prompt cannot be crossed by this reply.
    if state.latest_for_route(&route, now_secs()).as_deref() != Some(request_id.as_str()) {
        return false;
    }
    if let Err(error) = validate_pending_attach_for_source(
        &state,
        &request_id,
        &attach_identity,
        Some(&reply_source),
        "text_reply_consume",
    ) {
        state.remove(&request_id);
        drop(state);
        app_warn!(
            "channel",
            "ask_user",
            "Text ask_user reply lost attach ownership before consume ({}): {}",
            request_id,
            ha_core::logging::redact_sensitive(&error.to_string())
        );
        send_text_reply_feedback(msg, ask_user_source_mismatch(current_locale())).await;
        return true;
    }

    let lowered = text.to_lowercase();
    if lowered == "cancel" {
        let Some(_) = state.remove(&request_id) else {
            return false;
        };
        drop(state);
        if let Err(error) =
            attach_identity.validate_source_live(Some(&reply_source), "text_reply_cancel")
        {
            app_warn!(
                "channel",
                "ask_user",
                "Skipped ask_user cancellation after attach changed ({}): {}",
                request_id,
                ha_core::logging::redact_sensitive(&error.to_string())
            );
            send_text_reply_feedback(msg, ask_user_source_mismatch(current_locale())).await;
            return true;
        }
        ask_user_mod::cancel_pending_ask_user_question(&request_id).await;
        return true;
    }

    if lowered == "done" {
        let complete = state
            .by_request
            .get(&request_id)
            .is_some_and(PendingAskUser::is_complete);
        if !complete {
            // Consume the control word but retain the canonical pending group.
            // Return a localized explanation through the same IM route instead
            // of submitting an incomplete group or leaking "done" as a turn.
            drop(state);
            send_text_reply_feedback(msg, ask_user_callback_incomplete(current_locale())).await;
            return true;
        }
        let Some(pending) = state.remove(&request_id) else {
            return false;
        };
        drop(state);
        if let Err(error) =
            attach_identity.validate_source_live(Some(&reply_source), "text_reply_submit")
        {
            app_warn!(
                "channel",
                "ask_user",
                "Skipped ask_user submit after attach changed ({}): {}",
                request_id,
                ha_core::logging::redact_sensitive(&error.to_string())
            );
            send_text_reply_feedback(msg, ask_user_source_mismatch(current_locale())).await;
            return true;
        }
        let answers = pending.into_answers();
        if let Err(e) = ask_user_mod::submit_ask_user_question_response(&request_id, answers).await
        {
            app_warn!(
                "channel",
                "ask_user",
                "Failed to submit ask_user answers ({}): {}",
                request_id,
                e
            );
        }
        return true;
    }

    let Some(current) = state.by_request.get_mut(&request_id) else {
        return false;
    };

    // Try to parse option markers. A reply like "1a,1c" splits into markers.
    let mut parsed_any = false;
    for token in text.split(|c: char| c == ',' || c.is_whitespace()) {
        let tok = token.trim();
        if tok.is_empty() || tok.eq_ignore_ascii_case("done") || tok.eq_ignore_ascii_case("cancel")
        {
            continue;
        }
        if let Some((qi, oi)) = parse_marker(tok) {
            if qi < current.group.questions.len() {
                let q = &current.group.questions[qi];
                if oi < q.options.len() {
                    let value = q.options[oi].value.clone();
                    let prog = current.progress.entry(q.question_id.clone()).or_default();
                    if q.multi_select {
                        if !prog.selected.contains(&value) {
                            prog.selected.push(value);
                        }
                    } else {
                        prog.selected = vec![value];
                        prog.custom_input = None;
                    }
                    parsed_any = true;
                }
            }
        }
    }

    // If no marker was parsed, route the whole text through the same Other /
    // free-text progress used by button callbacks.
    if !parsed_any {
        parsed_any = apply_text_custom_input(current, &text);
    }

    if !parsed_any {
        return false;
    }

    let should_finish =
        !current.group.questions.iter().any(|q| q.multi_select) && current.is_complete();
    let pending_for_submit = if should_finish {
        state.remove(&request_id)
    } else {
        None
    };
    drop(state);

    if let Some(pending) = pending_for_submit {
        if let Err(error) =
            attach_identity.validate_source_live(Some(&reply_source), "text_reply_submit")
        {
            app_warn!(
                "channel",
                "ask_user",
                "Skipped ask_user submit after attach changed ({}): {}",
                request_id,
                ha_core::logging::redact_sensitive(&error.to_string())
            );
            send_text_reply_feedback(msg, ask_user_source_mismatch(current_locale())).await;
            return true;
        }
        let answers = pending.into_answers();
        if let Err(e) = ask_user_mod::submit_ask_user_question_response(&request_id, answers).await
        {
            app_warn!(
                "channel",
                "ask_user",
                "Failed to submit ask_user answers ({}): {}",
                request_id,
                e
            );
        }
    }

    true
}

/// Parse an option marker like "1a" or "10c" → (question_index, option_index).
fn parse_marker(tok: &str) -> Option<(usize, usize)> {
    let tok = tok.trim().to_lowercase();
    if tok.len() < 2 {
        return None;
    }
    let letter = tok.chars().last().filter(|c| c.is_ascii_alphabetic())?;
    let oi = (letter as u8 - b'a') as usize;
    let number = tok.strip_suffix(letter).unwrap_or(tok.as_str());
    let qi: usize = number.parse().ok()?;
    if qi == 0 {
        return None;
    }
    Some((qi - 1, oi))
}

// ── Callback handler (button-capable channels) ────────────────────

fn parse_file_callback_id(data: &str, marker: &str) -> anyhow::Result<(String, usize)> {
    let rest = data
        .strip_prefix(ASK_USER_PREFIX)
        .ok_or_else(|| anyhow::anyhow!("Not an ask_user callback"))?;
    let (request_id, question_index) = rest
        .rsplit_once(marker)
        .ok_or_else(|| anyhow::anyhow!("Not an ask_user file callback"))?;
    if request_id.is_empty() || request_id.contains(':') {
        anyhow::bail!("Invalid ask_user file request id");
    }
    let question_index = parse_callback_index(question_index, "file question index")?;
    Ok((request_id.to_string(), question_index))
}

pub fn is_ask_user_file_open_callback(data: &str) -> bool {
    parse_file_callback_id(data, ":f:").is_ok()
}

pub fn is_ask_user_file_modal_submit(data: &str) -> bool {
    parse_file_callback_id(data, ":fm:").is_ok()
}

async fn resolve_discord_file_callback(
    callback_data: &str,
    marker: &str,
    callback_source: &InteractiveCallbackSource,
    source: &'static str,
) -> anyhow::Result<(String, usize, AskUserFileConstraints, String)> {
    if callback_source.channel_id != ChannelId::Discord {
        anyhow::bail!("File upload modal is Discord-only");
    }
    let account_enabled = {
        let store = ha_core::config::cached_config();
        store
            .channels
            .find_account(&callback_source.account_id)
            .is_some_and(|account| account.discord_file_requests_enabled())
    };
    if !account_enabled {
        anyhow::bail!("Discord file requests are disabled for this account");
    }
    let (request_id, question_index) = parse_file_callback_id(callback_data, marker)?;
    let (attach_identity, constraints, question_text) = {
        let mut state = get_pending_state().lock().await;
        if state.is_expired(&request_id, now_secs()) {
            state.remove(&request_id);
            anyhow::bail!("The file request has expired");
        }
        let pending = state
            .by_request
            .get(&request_id)
            .ok_or_else(|| anyhow::anyhow!("No pending file request"))?;
        let question = pending
            .group
            .questions
            .get(question_index)
            .ok_or_else(|| anyhow::anyhow!("File question index is out of range"))?;
        let constraints = pending_file_constraints(question)
            .ok_or_else(|| anyhow::anyhow!("The target question is not a file request"))?;
        (
            pending.attach_identity.clone(),
            constraints,
            question.text.fallback_text().to_string(),
        )
    };
    attach_identity.validate_source_live(Some(callback_source), source)?;
    Ok((request_id, question_index, constraints, question_text))
}

/// Compile the shared file request into Discord's Modal + Label + File Upload
/// component shape. The custom IDs carry only the request/question identity;
/// provider-resolved attachment metadata is revalidated on submit.
pub async fn build_discord_file_modal(
    callback_data: &str,
    callback_source: &InteractiveCallbackSource,
) -> anyhow::Result<serde_json::Value> {
    let (request_id, question_index, constraints, question_text) = resolve_discord_file_callback(
        callback_data,
        ":f:",
        callback_source,
        "discord_file_modal_open",
    )
    .await?;
    let file_types = constraints
        .types
        .iter()
        .filter_map(|value| match value.as_str() {
            "application/pdf" => Some(".pdf"),
            "text/plain" => Some(".txt"),
            "text/markdown" => Some(".md"),
            _ => None,
        })
        .collect::<Vec<_>>();
    Ok(serde_json::json!({
        "custom_id": format!("{ASK_USER_PREFIX}{request_id}:fm:{question_index}"),
        "title": "Upload requested file",
        "components": [{
            "type": 18,
            "label": "Choose one file",
            "description": ha_core::truncate_utf8(&question_text, 100),
            "component": {
                "type": 19,
                "custom_id": format!("{ASK_USER_PREFIX}{request_id}:fu:{question_index}"),
                "min_values": 1,
                "max_values": 1,
                "required": true,
                "file_types": file_types,
            }
        }]
    }))
}

pub async fn validate_discord_file_modal_submit(
    callback_data: &str,
    callback_source: &InteractiveCallbackSource,
) -> anyhow::Result<()> {
    resolve_discord_file_callback(
        callback_data,
        ":fm:",
        callback_source,
        "discord_file_modal_submit",
    )
    .await
    .map(|_| ())
}

/// Check whether a callback data string belongs to an ask_user flow.
pub fn is_ask_user_callback(data: &str) -> bool {
    data.starts_with(ASK_USER_PREFIX)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum QuestionLookup {
    Index(usize),
    LegacyId(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum OptionLookup {
    Index(usize),
    LegacyValue(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AskUserCallbackAction {
    Select {
        question: QuestionLookup,
        option: OptionLookup,
    },
    Done {
        question: QuestionLookup,
    },
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedAskUserCallback {
    request_id: String,
    action: AskUserCallbackAction,
}

fn parse_callback_index(raw: &str, field: &str) -> anyhow::Result<usize> {
    raw.parse::<usize>()
        .map_err(|_| anyhow::anyhow!("Invalid {field}: {raw}"))
}

/// Parse the compact index protocol and retain read compatibility with button
/// messages sent by an older process during a rolling restart.
fn parse_ask_user_callback(callback_data: &str) -> anyhow::Result<ParsedAskUserCallback> {
    let rest = callback_data
        .strip_prefix(ASK_USER_PREFIX)
        .ok_or_else(|| anyhow::anyhow!("Not an ask_user callback"))?;
    let mut head = rest.splitn(3, ':');
    let request_id = head
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Missing request_id"))?
        .to_string();
    let action = head
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Missing action"))?;
    let tail = head.next();

    let action = match action {
        "c" | "cancel" if tail.is_none() => AskUserCallbackAction::Cancel,
        "s" => {
            let (question, option) = tail
                .and_then(|tail| tail.split_once(':'))
                .ok_or_else(|| anyhow::anyhow!("Missing question/option index"))?;
            AskUserCallbackAction::Select {
                question: QuestionLookup::Index(parse_callback_index(question, "question index")?),
                option: OptionLookup::Index(parse_callback_index(option, "option index")?),
            }
        }
        "d" => AskUserCallbackAction::Done {
            question: QuestionLookup::Index(parse_callback_index(
                tail.ok_or_else(|| anyhow::anyhow!("Missing question index"))?,
                "question index",
            )?),
        },
        "select" => {
            let (question_id, option_value) = tail
                .and_then(|tail| tail.split_once(':'))
                .ok_or_else(|| anyhow::anyhow!("Missing legacy question/option value"))?;
            if question_id.is_empty() || option_value.is_empty() {
                return Err(anyhow::anyhow!(
                    "Legacy question_id and option_value must be non-empty"
                ));
            }
            AskUserCallbackAction::Select {
                question: QuestionLookup::LegacyId(question_id.to_string()),
                option: OptionLookup::LegacyValue(option_value.to_string()),
            }
        }
        "done" => AskUserCallbackAction::Done {
            question: QuestionLookup::LegacyId(
                tail.filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("Missing legacy question_id"))?
                    .to_string(),
            ),
        },
        _ => return Err(anyhow::anyhow!("Unknown ask_user action: {action}")),
    };

    Ok(ParsedAskUserCallback { request_id, action })
}

fn resolve_question_index(
    pending: &PendingAskUser,
    lookup: &QuestionLookup,
) -> anyhow::Result<usize> {
    match lookup {
        QuestionLookup::Index(index) => {
            if *index >= pending.group.questions.len() {
                return Err(anyhow::anyhow!(
                    "ask_user question index {} is out of range (questions={})",
                    index,
                    pending.group.questions.len()
                ));
            }
            Ok(*index)
        }
        QuestionLookup::LegacyId(question_id) => pending
            .group
            .questions
            .iter()
            .position(|question| question.question_id == *question_id)
            .ok_or_else(|| anyhow::anyhow!("Unknown ask_user question_id {question_id}")),
    }
}

fn resolve_option_index(
    pending: &PendingAskUser,
    question_index: usize,
    lookup: &OptionLookup,
) -> anyhow::Result<usize> {
    let question = pending.group.questions.get(question_index).ok_or_else(|| {
        anyhow::anyhow!("ask_user question index {question_index} is out of range")
    })?;
    match lookup {
        OptionLookup::Index(index) => {
            if *index >= question.options.len() {
                return Err(anyhow::anyhow!(
                    "ask_user option index {} is out of range for question {} (options={})",
                    index,
                    question_index,
                    question.options.len()
                ));
            }
            Ok(*index)
        }
        OptionLookup::LegacyValue(option_value) => question
            .options
            .iter()
            .position(|option| option.value == *option_value)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Unknown ask_user option value for question {}",
                    question.question_id
                )
            }),
    }
}

fn apply_callback_selection(
    pending: &mut PendingAskUser,
    question_lookup: &QuestionLookup,
    option_lookup: &OptionLookup,
) -> anyhow::Result<()> {
    let question_index = resolve_question_index(pending, question_lookup)?;
    let option_index = resolve_option_index(pending, question_index, option_lookup)?;
    let question = &pending.group.questions[question_index];
    let question_id = question.question_id.clone();
    let option_value = question.options[option_index].value.clone();
    let multi_select = question.multi_select;
    let progress = pending.progress.entry(question_id).or_default();
    if multi_select {
        if progress.selected.contains(&option_value) {
            progress.selected.retain(|value| value != &option_value);
        } else {
            progress.selected.push(option_value);
        }
    } else {
        progress.selected = vec![option_value];
        progress.custom_input = None;
    }
    Ok(())
}

fn validate_done_question(
    pending: &PendingAskUser,
    question_lookup: &QuestionLookup,
) -> anyhow::Result<()> {
    let question_index = resolve_question_index(pending, question_lookup)?;
    let question = &pending.group.questions[question_index];
    if !question.multi_select {
        return Err(anyhow::anyhow!(
            "ask_user Done is valid only for a multi-select question"
        ));
    }
    Ok(())
}

/// Parse a compact `ask_user:{request_id}:s:{question_index}:{option_index}` /
/// `:d:{question_index}` / `:c` callback (plus the legacy id/value format),
/// update canonical pending state, and submit only when the whole group is
/// complete.
///
/// Returns a short human-readable label for UI feedback.
pub async fn handle_ask_user_callback_with_source(
    callback_data: &str,
    callback_source: Option<InteractiveCallbackSource>,
    source: &'static str,
) -> anyhow::Result<String> {
    let parsed = parse_ask_user_callback(callback_data)?;
    let request_id = parsed.request_id;

    // Compact and legacy callbacks share the same fail-closed source boundary.
    // A missing source is not accepted: live ask_user prompts are short-lived,
    // so Telegram's >48h callback-without-message compatibility is irrelevant.
    let attach_identity = {
        let state = get_pending_state().lock().await;
        state
            .by_request
            .get(&request_id)
            .map(|pending| pending.attach_identity.clone())
            .ok_or_else(|| anyhow::anyhow!("No pending ask_user with id {}", request_id))?
    };
    attach_identity.validate_source_live(callback_source.as_ref(), source)?;

    // Defense-in-depth: if the group's timeout has elapsed but the tool-side
    // cleanup hasn't run yet, drop the stale pending entry and surface a clear
    // error rather than mutating state nobody is listening on.
    if drop_if_expired(&request_id).await {
        return Err(anyhow::anyhow!(
            "ask_user group {} already timed out",
            request_id
        ));
    }

    let locale = current_locale();
    match parsed.action {
        AskUserCallbackAction::Cancel => {
            let pending = {
                let mut state = get_pending_state().lock().await;
                validate_pending_attach_for_source(
                    &state,
                    &request_id,
                    &attach_identity,
                    callback_source.as_ref(),
                    source,
                )?;
                state.remove(&request_id)
            };
            if pending.is_none() {
                return Err(anyhow::anyhow!(
                    "No pending ask_user with id {}",
                    request_id
                ));
            }
            attach_identity.validate_source_live(callback_source.as_ref(), source)?;
            ask_user_mod::cancel_pending_ask_user_question(&request_id).await;
            Ok(ask_user_callback_cancelled(locale).to_string())
        }
        AskUserCallbackAction::Select { question, option } => {
            let (should_submit, pending_for_submit) = {
                let mut state = get_pending_state().lock().await;
                let Some(pending) = state.by_request.get_mut(&request_id) else {
                    return Err(anyhow::anyhow!(
                        "No pending ask_user with id {}",
                        request_id
                    ));
                };
                if pending.attach_identity != attach_identity {
                    return Err(anyhow::anyhow!(
                        "ask_user attach identity changed for request {}",
                        request_id
                    ));
                }
                attach_identity.validate_source_live(callback_source.as_ref(), source)?;
                apply_callback_selection(pending, &question, &option)?;
                // Single-select complete → submit; multi-select waits for "done".
                let has_multi = pending.group.questions.iter().any(|q| q.multi_select);
                let should_submit = !has_multi && pending.is_complete();
                if should_submit {
                    let pending = state.remove(&request_id);
                    (true, pending)
                } else {
                    (false, None)
                }
            };

            if should_submit {
                if let Some(pending) = pending_for_submit {
                    attach_identity.validate_source_live(callback_source.as_ref(), source)?;
                    let answers = pending.into_answers();
                    ask_user_mod::submit_ask_user_question_response(&request_id, answers).await?;
                    return Ok(ask_user_callback_answered(locale).to_string());
                }
            }
            Ok(ask_user_callback_selected(locale).to_string())
        }
        AskUserCallbackAction::Done { question } => {
            let pending = {
                let mut state = get_pending_state().lock().await;
                let Some(pending) = state.by_request.get(&request_id) else {
                    return Err(anyhow::anyhow!(
                        "No pending ask_user with id {}",
                        request_id
                    ));
                };
                if pending.attach_identity != attach_identity {
                    return Err(anyhow::anyhow!(
                        "ask_user attach identity changed for request {}",
                        request_id
                    ));
                }
                attach_identity.validate_source_live(callback_source.as_ref(), source)?;
                validate_done_question(pending, &question)?;
                if !pending.is_complete() {
                    return Ok(ask_user_callback_incomplete(locale).to_string());
                }
                state
                    .remove(&request_id)
                    .ok_or_else(|| anyhow::anyhow!("No pending ask_user with id {}", request_id))?
            };
            attach_identity.validate_source_live(callback_source.as_ref(), source)?;
            let answers = pending.into_answers();
            ask_user_mod::submit_ask_user_question_response(&request_id, answers).await?;
            Ok(ask_user_callback_answered(locale).to_string())
        }
    }
}

pub fn spawn_callback_handler_with_source(
    data: &str,
    source: &'static str,
    callback_source: Option<InteractiveCallbackSource>,
) {
    let data = data.to_string();
    tokio::spawn(async move {
        match handle_ask_user_callback_with_source(&data, callback_source, source).await {
            Ok(label) => app_info!("channel", source, "ask_user: {}", label),
            Err(e) => app_warn!("channel", source, "ask_user callback failed: {}", e),
        }
    });
}

/// Unified interactive-callback dispatcher for channel plugins.
///
/// Detects whether a callback string belongs to an approval or ask_user flow
/// and spawns the corresponding handler. Returns `true` if the callback was
/// consumed (the plugin should not treat it as a regular message).
pub fn try_dispatch_interactive_callback(
    data: &str,
    source: &'static str,
    callback_source: Option<InteractiveCallbackSource>,
) -> bool {
    if super::approval::is_approval_callback(data) {
        super::approval::spawn_callback_handler_with_source(data, source, callback_source);
        return true;
    }
    if is_ask_user_callback(data) {
        spawn_callback_handler_with_source(data, source, callback_source);
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{
        apply_callback_selection, apply_text_custom_input, build_buttons_for_locale,
        buttons_fit_callback_limit, format_prompt_for_locale, parse_ask_user_callback,
        parse_file_callback_id, parse_marker, text_reply_hint_for_locale, validate_done_question,
        AskUserCallbackAction, InteractiveAttachIdentity, InteractiveCallbackSource, OptionLookup,
        PendingAskUser, PendingAskUserState, QuestionLookup, ASK_USER_CALLBACK_MAX_BYTES,
    };
    use ha_core::ask_user::AskUserQuestionGroup;
    use ha_core::channel::db::ChannelConversation;
    use ha_core::channel::types::ChannelId;

    fn sample_conversation(attach_id: i64) -> ChannelConversation {
        ChannelConversation {
            id: attach_id,
            channel_id: "telegram".to_string(),
            account_id: "account".to_string(),
            chat_id: "chat".to_string(),
            thread_id: None,
            session_id: "session-1".to_string(),
            sender_id: None,
            sender_tenant_id: None,
            sender_name: None,
            chat_type: "dm".to_string(),
            source: "inbound".to_string(),
            attached_at: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    fn sample_attach_identity() -> InteractiveAttachIdentity {
        InteractiveAttachIdentity::from_conversation("session-1", &sample_conversation(7))
            .expect("sample attach identity")
    }

    fn sample_group() -> AskUserQuestionGroup {
        serde_json::from_value(serde_json::json!({
            "requestId": format!("auq_{}", "a".repeat(32)),
            "sessionId": "session-1",
            "questions": [
                {
                    "questionId": "question-id-that-must-not-enter-the-callback-payload",
                    "text": "Choose any",
                    "options": [{
                        "value": "option-value-that-must-not-enter-the-callback-payload",
                        "label": "A"
                    }],
                    "allowCustom": true,
                    "multiSelect": true
                },
                {
                    "questionId": "q2",
                    "text": "Choose one",
                    "options": [{ "value": "b", "label": "B" }],
                    "allowCustom": true,
                    "multiSelect": false
                }
            ]
        }))
        .expect("sample ask_user group must deserialize")
    }

    fn sample_file_group(request_id: &str, types: &[&str]) -> AskUserQuestionGroup {
        serde_json::from_value(serde_json::json!({
            "requestId": request_id,
            "sessionId": "session-1",
            "questions": [{
                "questionId": "file-question",
                "text": "Upload a document",
                "options": [],
                "allowCustom": false,
                "multiSelect": false,
                "inputKind": "file",
                "fileConstraints": {
                    "types": types,
                    "maxBytes": 10 * 1024 * 1024,
                    "count": 1
                }
            }]
        }))
        .expect("sample file group must deserialize")
    }

    #[test]
    fn parse_marker_rejects_unicode_without_panicking() {
        assert_eq!(parse_marker("你好"), None);
        assert_eq!(parse_marker("1好"), None);
        assert_eq!(parse_marker("10c"), Some((9, 2)));
    }

    #[test]
    fn compact_callbacks_use_indices_and_fit_telegram_limit() {
        let group = sample_group();
        let buttons = build_buttons_for_locale(&group, "en-US");
        assert!(buttons_fit_callback_limit(&buttons));
        for callback in buttons
            .iter()
            .flatten()
            .filter_map(|button| button.callback_data.as_deref())
        {
            assert!(callback.len() <= ASK_USER_CALLBACK_MAX_BYTES);
            assert!(!callback.contains("question-id-that"));
            assert!(!callback.contains("option-value-that"));
        }
        assert!(buttons.iter().flatten().any(|button| {
            button.callback_data.as_deref()
                == Some("ask_user:auq_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:s:0:0")
        }));
    }

    #[test]
    fn callback_parser_accepts_compact_and_legacy_protocols() {
        let compact = parse_ask_user_callback("ask_user:req:s:1:2").unwrap();
        assert_eq!(
            compact.action,
            AskUserCallbackAction::Select {
                question: QuestionLookup::Index(1),
                option: OptionLookup::Index(2),
            }
        );

        let legacy = parse_ask_user_callback("ask_user:req:select:q2:b").unwrap();
        assert_eq!(
            legacy.action,
            AskUserCallbackAction::Select {
                question: QuestionLookup::LegacyId("q2".to_string()),
                option: OptionLookup::LegacyValue("b".to_string()),
            }
        );
    }

    #[test]
    fn file_callback_ids_are_compact_and_reject_ambiguous_request_ids() {
        let (request_id, question_index) =
            parse_file_callback_id("ask_user:auq_123:f:0", ":f:").unwrap();
        assert_eq!(request_id, "auq_123");
        assert_eq!(question_index, 0);
        assert!(parse_file_callback_id("ask_user:bad:id:f:0", ":f:").is_err());
        assert!(parse_file_callback_id("ask_user:auq_123:f:nope", ":f:").is_err());
    }

    #[test]
    fn callback_indices_fail_closed_and_done_requires_complete_group() {
        let mut pending = PendingAskUser::new(sample_group(), sample_attach_identity());
        assert!(apply_callback_selection(
            &mut pending,
            &QuestionLookup::Index(99),
            &OptionLookup::Index(0)
        )
        .is_err());
        assert!(apply_callback_selection(
            &mut pending,
            &QuestionLookup::Index(0),
            &OptionLookup::Index(99)
        )
        .is_err());

        apply_callback_selection(
            &mut pending,
            &QuestionLookup::Index(0),
            &OptionLookup::Index(0),
        )
        .unwrap();
        validate_done_question(&pending, &QuestionLookup::Index(0)).unwrap();
        assert!(!pending.is_complete());

        apply_callback_selection(
            &mut pending,
            &QuestionLookup::Index(1),
            &OptionLookup::Index(0),
        )
        .unwrap();
        assert!(pending.is_complete());
        assert!(validate_done_question(&pending, &QuestionLookup::Index(1)).is_err());
    }

    #[test]
    fn button_and_text_paths_share_progress_and_terminal_removal() {
        let mut group = sample_group();
        group.questions.truncate(1);
        let request_id = group.request_id.clone();
        let route = (
            "telegram".to_string(),
            "account".to_string(),
            "chat".to_string(),
            None,
        );
        let mut state = PendingAskUserState::default();
        state
            .insert(
                route.clone(),
                PendingAskUser::new(group, sample_attach_identity()),
                0,
            )
            .expect("ordinary prompt should register");

        let pending = state.by_request.get_mut(&request_id).unwrap();
        apply_callback_selection(pending, &QuestionLookup::Index(0), &OptionLookup::Index(0))
            .unwrap();
        assert!(apply_text_custom_input(pending, "Other value"));
        let progress = pending.progress.values().next().unwrap();
        assert_eq!(progress.selected.len(), 1);
        assert_eq!(progress.custom_input.as_deref(), Some("Other value"));

        assert_eq!(
            state.latest_for_route(&route, 0).as_deref(),
            Some(request_id.as_str())
        );
        assert!(state.remove(&request_id).is_some());
        assert!(state.by_request.is_empty());
        assert!(state.by_route.is_empty());
    }

    #[test]
    fn file_prompt_and_hint_render_only_the_allowed_types() {
        let group = sample_file_group("file-pdf", &["application/pdf"]);

        let prompt = format_prompt_for_locale(&group, "en-US");
        assert!(prompt.contains("(PDF, max 10 MiB)"));
        assert!(!prompt.contains("TXT"));
        assert!(!prompt.contains("MD"));

        let hint = text_reply_hint_for_locale(&group, "en-US");
        assert!(hint.contains("next PDF file"));
        assert!(!hint.contains("TXT"));
        assert!(!hint.contains("MD"));
    }

    #[test]
    fn one_route_rejects_a_second_live_file_request() {
        let route = (
            "telegram".to_string(),
            "account".to_string(),
            "chat".to_string(),
            None,
        );
        let first_request_id = "file-first";
        let mut state = PendingAskUserState::default();
        state
            .insert(
                route.clone(),
                PendingAskUser::new(
                    sample_file_group(first_request_id, &["application/pdf"]),
                    sample_attach_identity(),
                ),
                0,
            )
            .expect("first file request should register");

        let conflict = state
            .insert(
                route,
                PendingAskUser::new(
                    sample_file_group("file-second", &["text/plain"]),
                    sample_attach_identity(),
                ),
                0,
            )
            .expect_err("second live file request must be rejected");

        assert_eq!(conflict, first_request_id);
        assert!(state.by_request.contains_key(first_request_id));
        assert!(!state.by_request.contains_key("file-second"));
    }

    #[test]
    fn ask_user_callback_requires_source_context() {
        let error = sample_attach_identity()
            .validate_source(None, "test")
            .expect_err("missing callback source must fail closed");
        assert!(error.to_string().contains("missing source context"));
    }

    #[test]
    fn attach_identity_binds_row_id_and_exact_callback_route() {
        let identity = sample_attach_identity();
        assert!(identity.matches_conversation(&sample_conversation(7)));
        assert!(!identity.matches_conversation(&sample_conversation(8)));

        let source = InteractiveCallbackSource::new(ChannelId::Telegram, "account", "chat", None);
        identity
            .validate_source(Some(&source), "test")
            .expect("exact route should validate");
        let wrong_chat =
            InteractiveCallbackSource::new(ChannelId::Telegram, "account", "other", None);
        assert!(identity.validate_source(Some(&wrong_chat), "test").is_err());
    }
}
