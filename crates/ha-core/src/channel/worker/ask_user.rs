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

use crate::ask_user::{
    self as ask_user_mod, AskUserQuestionAnswer, AskUserQuestionGroup, AskUserTimedOutPayload,
};
use crate::channel::db::ChannelDB;
use crate::channel::registry::ChannelRegistry;
use crate::channel::types::{ChannelId, InlineButton, ReplyPayload};

/// Callback data prefix for ask_user buttons across all channels.
pub(crate) const ASK_USER_PREFIX: &str = "ask_user:";

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

pub fn validate_callback_source_for_session(
    session_id: &str,
    callback_source: Option<&InteractiveCallbackSource>,
    source: &'static str,
) -> anyhow::Result<()> {
    let Some(callback_source) = callback_source else {
        // Permissive on a missing source — Telegram no-message callbacks (>48h-old
        // / inline buttons) legitimately carry no chat context and can't be
        // validated. This is acceptable for the lower-risk ask_user Q&A path; the
        // security-sensitive *approval* path fails closed at its own caller
        // (`handle_approval_callback_with_source`, MISC-11) and never passes None
        // here, so this branch can't weaken approvals.
        return Ok(());
    };
    let channel_db = crate::globals::get_channel_db()
        .ok_or_else(|| anyhow::anyhow!("ChannelDB not initialized for IM callback validation"))?;
    let Some(conversation) = channel_db.get_conversation_by_session(session_id)? else {
        return Err(anyhow::anyhow!(
            "No channel conversation attached to session {}",
            session_id
        ));
    };

    let expected_channel_id = callback_source.channel_id.to_string();
    let expected_thread = normalized_thread(conversation.thread_id.as_deref());
    let source_thread = normalized_thread(callback_source.thread_id.as_deref());
    if conversation.channel_id != expected_channel_id
        || conversation.account_id != callback_source.account_id
        || conversation.chat_id != callback_source.chat_id
        || expected_thread != source_thread
    {
        return Err(anyhow::anyhow!(
            "Interactive callback source mismatch from {}: expected {}:{}:{}:{:?}, got {}:{}:{}:{:?}",
            source,
            conversation.channel_id,
            conversation.account_id,
            conversation.chat_id,
            expected_thread,
            expected_channel_id,
            callback_source.account_id,
            callback_source.chat_id,
            source_thread,
        ));
    }

    Ok(())
}

// ── Pending state for in-progress IM answers ─────────────────────

/// One question's in-progress answer accumulator (button channels only need
/// selected values; multi-select and text fallbacks use the same state).
#[derive(Debug, Clone, Default)]
struct QuestionProgress {
    selected: Vec<String>,
    custom_input: Option<String>,
}

#[derive(Debug, Clone)]
struct PendingAskUser {
    request_id: String,
    group: AskUserQuestionGroup,
    progress: HashMap<String, QuestionProgress>,
}

impl PendingAskUser {
    fn new(group: AskUserQuestionGroup) -> Self {
        let mut progress = HashMap::new();
        for q in &group.questions {
            progress.insert(q.question_id.clone(), QuestionProgress::default());
        }
        Self {
            request_id: group.request_id.clone(),
            group,
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
            !prog.selected.is_empty() || prog.custom_input.is_some()
        })
    }
}

/// Pending button-based ask_user groups keyed by request_id.
static BUTTON_PENDING: OnceLock<Mutex<HashMap<String, PendingAskUser>>> = OnceLock::new();

fn get_button_pending() -> &'static Mutex<HashMap<String, PendingAskUser>> {
    BUTTON_PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Pending text-reply ask_user groups keyed by (account_id, chat_id) — LIFO.
static TEXT_PENDING: OnceLock<Mutex<HashMap<(String, String), Vec<PendingAskUser>>>> =
    OnceLock::new();

fn get_text_pending() -> &'static Mutex<HashMap<(String, String), Vec<PendingAskUser>>> {
    TEXT_PENDING.get_or_init(|| Mutex::new(HashMap::new()))
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
    let mut map = get_button_pending().lock().await;
    let expired = map
        .get(request_id)
        .and_then(|p| p.group.timeout_at)
        .map(|t| t > 0 && now >= t)
        .unwrap_or(false);
    if expired {
        map.remove(request_id);
    }
    expired
}

/// Remove any in-memory pending state for the given request_id from both the
/// button and text-reply maps. Called by the tool execution path when a
/// question group is cancelled, timed out, or answered through a non-IM
/// channel, so stale entries don't accumulate.
pub async fn drop_pending_by_request_id(request_id: &str) {
    {
        let mut map = get_button_pending().lock().await;
        map.remove(request_id);
    }
    {
        let mut map = get_text_pending().lock().await;
        let mut empty_keys = Vec::new();
        for (key, list) in map.iter_mut() {
            list.retain(|p| p.request_id != request_id);
            if list.is_empty() {
                empty_keys.push(key.clone());
            }
        }
        for k in empty_keys {
            map.remove(&k);
        }
    }
}

/// Remove every pending ask_user entry owned by a deleted/purged session.
pub async fn drop_pending_for_session(session_id: &str) {
    {
        let mut map = get_button_pending().lock().await;
        map.retain(|_, pending| pending.group.session_id != session_id);
    }
    {
        let mut map = get_text_pending().lock().await;
        map.retain(|_, list| {
            list.retain(|pending| pending.group.session_id != session_id);
            !list.is_empty()
        });
    }
}

// ── Button / prompt rendering ─────────────────────────────────────

fn tr(locale: &str, row: [&'static str; 12]) -> &'static str {
    crate::i18n::pick_locale(locale, row)
}

#[cfg(not(test))]
fn current_locale() -> &'static str {
    crate::i18n::current_ui_locale()
}

#[cfg(test)]
fn current_locale() -> &'static str {
    crate::i18n::DEFAULT_LOCALE
}

/// Render the prompt text for a group. Includes context and all questions with
/// their options numbered so the user can reference them either via button or
/// text reply. Each field is individually truncated, and the full prompt is
/// clamped to ~3500 bytes so it fits inside the strictest IM payload limit
/// (Discord 2000 / Telegram 4096 / Slack 3000 / LINE 5000).
fn format_prompt(group: &AskUserQuestionGroup) -> String {
    format_prompt_for_locale(group, current_locale())
}

fn format_prompt_for_locale(group: &AskUserQuestionGroup, locale: &str) -> String {
    let mut out = String::new();
    out.push_str(question_from_ai_title(locale));
    out.push('\n');
    if let Some(ctx) = &group.context {
        out.push('\n');
        out.push_str(crate::truncate_utf8(ctx.fallback_text(), 500));
        out.push('\n');
    }
    for (qi, q) in group.questions.iter().enumerate() {
        let qtext = crate::truncate_utf8(q.text.fallback_text(), 500);
        out.push_str(&format!("\n{}. {}", qi + 1, qtext));
        if q.multi_select {
            out.push_str(multi_select_suffix(locale));
        }
        out.push('\n');
        for (oi, opt) in q.options.iter().enumerate() {
            let marker = option_marker(qi, oi);
            let rec = if opt.recommended { " ★" } else { "" };
            let label = crate::truncate_utf8(opt.label.fallback_text(), 100);
            out.push_str(&format!("  {marker}. {label}{rec}\n"));
            if let Some(desc) = &opt.description {
                let desc = crate::truncate_utf8(desc.fallback_text(), 200);
                out.push_str(&format!("     {desc}\n"));
            }
        }
    }
    crate::truncate_utf8(&out, 3500).to_string()
}

/// Build a marker like "1a" / "2b" for question `qi` option `oi`.
fn option_marker(qi: usize, oi: usize) -> String {
    let letter = (b'a' + oi as u8) as char;
    format!("{}{}", qi + 1, letter)
}

/// Extra hint text sent to channels without button support.
fn text_reply_hint(group: &AskUserQuestionGroup) -> String {
    text_reply_hint_for_locale(group, current_locale())
}

fn text_reply_hint_for_locale(group: &AskUserQuestionGroup, locale: &str) -> String {
    let has_multi = group.questions.iter().any(|q| q.multi_select);
    if has_multi {
        tr(
            locale,
            [
                "\n请用 `1a`（单选）或 `1a,1c`（多选）这样的选项标记回复。完成后输入 `done`。",
                "\n請用 `1a`（單選）或 `1a,1c`（多選）這樣的選項標記回覆。完成後輸入 `done`。",
                "\nReply with option markers like `1a` (single-select) or `1a,1c` (multi-select). Type `done` when finished.",
                "\n`1a`（単一選択）や `1a,1c`（複数選択）のような選択肢マーカーで返信してください。完了したら `done` と入力してください。",
                "\n`1a`(단일 선택) 또는 `1a,1c`(다중 선택) 같은 옵션 표시로 답장하세요. 완료되면 `done`을 입력하세요.",
                "\nResponde con marcadores de opción como `1a` (selección única) o `1a,1c` (selección múltiple). Escribe `done` al terminar.",
                "\nResponda com marcadores de opção como `1a` (seleção única) ou `1a,1c` (seleção múltipla). Digite `done` ao terminar.",
                "\nОтветьте маркерами вариантов вроде `1a` (один выбор) или `1a,1c` (несколько вариантов). Введите `done`, когда закончите.",
                "\nرد بعلامات الخيارات مثل `1a` (اختيار واحد) أو `1a,1c` (اختيارات متعددة). اكتب `done` عند الانتهاء.",
                "\n`1a` (tek seçim) veya `1a,1c` (çoklu seçim) gibi seçenek işaretleriyle yanıtlayın. Bitirince `done` yazın.",
                "\nTrả lời bằng ký hiệu lựa chọn như `1a` (chọn một) hoặc `1a,1c` (chọn nhiều). Nhập `done` khi hoàn tất.",
                "\nBalas dengan penanda pilihan seperti `1a` (pilihan tunggal) atau `1a,1c` (berbilang pilihan). Taip `done` apabila selesai.",
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

/// Build inline button rows for button-capable channels.
/// Each question's options form one row; multi-select questions get a
/// trailing "Done" button row.
fn build_buttons(group: &AskUserQuestionGroup) -> Vec<Vec<InlineButton>> {
    build_buttons_for_locale(group, current_locale())
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
                    "{}{}:select:{}:{}",
                    ASK_USER_PREFIX, group.request_id, q.question_id, opt.value
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
                callback_data: Some(format!(
                    "{}{}:done:{}",
                    ASK_USER_PREFIX, group.request_id, q.question_id
                )),
                url: None,
            }]);
        }
    }
    // Top-level cancel
    rows.push(vec![InlineButton {
        text: cancel_button_text(locale).to_string(),
        callback_data: Some(format!("{}{}:cancel", ASK_USER_PREFIX, group.request_id)),
        url: None,
    }]);
    rows
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
    let Some(bus) = crate::globals::get_event_bus() else {
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

            let store = crate::config::cached_config();
            let account_config = match store.channels.find_account(&conversation.account_id) {
                Some(c) => c.clone(),
                None => continue,
            };

            let channel_id: crate::channel::types::ChannelId = match serde_json::from_value(
                serde_json::Value::String(conversation.channel_id.clone()),
            ) {
                Ok(id) => id,
                Err(_) => continue,
            };

            let supports_buttons = registry
                .get_plugin(&channel_id)
                .map(|p| p.capabilities().supports_buttons)
                .unwrap_or(false);

            let prompt_text = format_prompt(&group);

            let payload = if supports_buttons {
                // Register pending state keyed by request_id.
                {
                    let mut pending = get_button_pending().lock().await;
                    pending.insert(group.request_id.clone(), PendingAskUser::new(group.clone()));
                }
                ReplyPayload {
                    text: Some(prompt_text),
                    buttons: build_buttons(&group),
                    thread_id: conversation.thread_id.clone(),
                    ..ReplyPayload::text("")
                }
            } else {
                // Register for text-reply routing.
                {
                    let key = (
                        conversation.account_id.clone(),
                        conversation.chat_id.clone(),
                    );
                    let mut pending = get_text_pending().lock().await;
                    pending
                        .entry(key)
                        .or_default()
                        .push(PendingAskUser::new(group.clone()));
                }
                let text = format!("{}{}", prompt_text, text_reply_hint(&group));
                ReplyPayload {
                    text: Some(text),
                    thread_id: conversation.thread_id.clone(),
                    ..ReplyPayload::text("")
                }
            };

            if let Err(e) = registry
                .send_reply(&account_config, &conversation.chat_id, &payload)
                .await
            {
                app_warn!(
                    "channel",
                    "ask_user",
                    "Failed to send ask_user prompt to channel: {}",
                    e
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

    let store = crate::config::cached_config();
    let locale = crate::i18n::effective_ui_locale(&store);
    let account_config = match store.channels.find_account(&conversation.account_id) {
        Some(c) => c.clone(),
        None => return,
    };

    let tag = id_tag(&event.request_id);
    let question = event
        .question_preview
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|s| format!("\n\n{}", crate::truncate_utf8(s.trim(), 500)));
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

// ── Text-reply handler (channels without buttons) ─────────────────

/// Try to interpret an inbound IM message as an ask_user text reply.
/// Returns `true` if the message was consumed.
///
/// Accepted reply formats:
/// - `1a`         single option for Q1
/// - `1a,1c`      multi-select for Q1
/// - `done`       finalise all answers (multi-select)
/// - `cancel`     abort the group
/// - `<text>`     free-form custom input for the first unanswered question
pub async fn try_handle_ask_user_reply(msg: &crate::channel::types::MsgContext) -> bool {
    let text = match msg.text.as_deref() {
        Some(t) => t.trim().to_string(),
        None => return false,
    };
    if text.is_empty() {
        return false;
    }

    let key = (msg.account_id.clone(), msg.chat_id.clone());
    let mut pending_map = get_text_pending().lock().await;
    let entry = match pending_map.get_mut(&key) {
        Some(v) if !v.is_empty() => v,
        _ => return false,
    };
    // Evict expired groups before operating — mirrors `drop_if_expired` for
    // the text-reply code path so a late reply can't re-animate a dead
    // question group when the tool-side cleanup is lagging.
    let now = now_secs();
    entry.retain(|p| p.group.timeout_at.map_or(true, |t| t == 0 || now < t));
    if entry.is_empty() {
        pending_map.remove(&key);
        return false;
    }
    // Operate on the most recent group (LIFO).
    let last_idx = entry.len() - 1;
    let current = &mut entry[last_idx];

    let lowered = text.to_lowercase();
    if lowered == "cancel" {
        let request_id = current.request_id.clone();
        entry.pop();
        if entry.is_empty() {
            pending_map.remove(&key);
        }
        drop(pending_map);
        ask_user_mod::cancel_pending_ask_user_question(&request_id).await;
        return true;
    }

    let should_finish =
        lowered == "done" || !current.group.questions.iter().any(|q| q.multi_select);

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
                    }
                    parsed_any = true;
                }
            }
        }
    }

    // If nothing parsed and there's exactly one question needing a custom answer,
    // treat the whole text as a custom input for the first unanswered question.
    if !parsed_any {
        if let Some(first_unanswered) = current.group.questions.iter().find(|q| {
            let prog = current
                .progress
                .get(&q.question_id)
                .cloned()
                .unwrap_or_default();
            prog.selected.is_empty() && prog.custom_input.is_none()
        }) {
            if first_unanswered.allow_custom {
                let qid = first_unanswered.question_id.clone();
                let prog = current.progress.entry(qid).or_default();
                prog.custom_input = Some(text.clone());
                parsed_any = true;
            }
        }
    }

    if !parsed_any {
        return false;
    }

    if should_finish && current.is_complete() {
        let request_id = current.request_id.clone();
        let Some(pending) = entry.pop() else {
            return false;
        };
        if entry.is_empty() {
            pending_map.remove(&key);
        }
        drop(pending_map);
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

/// Check whether a callback data string belongs to an ask_user flow.
pub fn is_ask_user_callback(data: &str) -> bool {
    data.starts_with(ASK_USER_PREFIX)
}

/// Parse an `ask_user:{request_id}:select:{question_id}:{option_value}` or
/// `ask_user:{request_id}:done:{question_id}` or `ask_user:{request_id}:cancel`
/// callback and update pending state / submit when complete.
///
/// Returns a short human-readable label for UI feedback.
pub async fn handle_ask_user_callback_with_source(
    callback_data: &str,
    callback_source: Option<InteractiveCallbackSource>,
    source: &'static str,
) -> anyhow::Result<String> {
    let rest = callback_data
        .strip_prefix(ASK_USER_PREFIX)
        .ok_or_else(|| anyhow::anyhow!("Not an ask_user callback"))?;

    let mut parts = rest.splitn(4, ':');
    let request_id = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("Missing request_id"))?
        .to_string();
    let action = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("Missing action"))?;

    if callback_source.is_some() {
        let session_id = {
            let map = get_button_pending().lock().await;
            map.get(&request_id)
                .map(|pending| pending.group.session_id.clone())
                .ok_or_else(|| anyhow::anyhow!("No pending ask_user with id {}", request_id))?
        };
        validate_callback_source_for_session(&session_id, callback_source.as_ref(), source)?;
    }

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
    match action {
        "cancel" => {
            get_button_pending().lock().await.remove(&request_id);
            ask_user_mod::cancel_pending_ask_user_question(&request_id).await;
            Ok(ask_user_callback_cancelled(locale).to_string())
        }
        "select" => {
            let question_id = parts
                .next()
                .ok_or_else(|| anyhow::anyhow!("Missing question_id"))?
                .to_string();
            let option_value = parts
                .next()
                .ok_or_else(|| anyhow::anyhow!("Missing option_value"))?
                .to_string();

            let (should_submit, pending_for_submit) = {
                let mut map = get_button_pending().lock().await;
                let Some(pending) = map.get_mut(&request_id) else {
                    return Err(anyhow::anyhow!(
                        "No pending ask_user with id {}",
                        request_id
                    ));
                };
                let q = pending
                    .group
                    .questions
                    .iter()
                    .find(|q| q.question_id == question_id)
                    .cloned();
                if let Some(q) = q {
                    let prog = pending.progress.entry(question_id.clone()).or_default();
                    if q.multi_select {
                        if prog.selected.contains(&option_value) {
                            prog.selected.retain(|v| v != &option_value);
                        } else {
                            prog.selected.push(option_value);
                        }
                    } else {
                        prog.selected = vec![option_value];
                    }
                }
                // Single-select complete → submit; multi-select waits for "done".
                let has_multi = pending.group.questions.iter().any(|q| q.multi_select);
                if !has_multi && pending.is_complete() {
                    let p = map.remove(&request_id);
                    (true, p)
                } else {
                    (false, None)
                }
            };

            if should_submit {
                if let Some(pending) = pending_for_submit {
                    let answers = pending.into_answers();
                    ask_user_mod::submit_ask_user_question_response(&request_id, answers).await?;
                    return Ok(ask_user_callback_answered(locale).to_string());
                }
            }
            Ok(ask_user_callback_selected(locale).to_string())
        }
        "done" => {
            let mut map = get_button_pending().lock().await;
            let Some(pending) = map.remove(&request_id) else {
                return Err(anyhow::anyhow!(
                    "No pending ask_user with id {}",
                    request_id
                ));
            };
            drop(map);
            let answers = pending.into_answers();
            ask_user_mod::submit_ask_user_question_response(&request_id, answers).await?;
            Ok(ask_user_callback_answered(locale).to_string())
        }
        _ => Err(anyhow::anyhow!("Unknown ask_user action: {}", action)),
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
    use super::parse_marker;

    #[test]
    fn parse_marker_rejects_unicode_without_panicking() {
        assert_eq!(parse_marker("你好"), None);
        assert_eq!(parse_marker("1好"), None);
        assert_eq!(parse_marker("10c"), Some((9, 2)));
    }
}
