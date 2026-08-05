//! Telegram Bot API 10.1/10.2 native rich reply lifecycle.
//!
//! Drafts are ephemeral 30-second snapshots. They are deliberately isolated
//! from the persistent final mutation: [`TelegramReplyStream::commit`] calls
//! the same final delivery path as a standalone rich reply and never relies on
//! draft state.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ha_core::channel::traits::ChannelReplyStream;
use ha_core::channel::types::{
    ChatType, InlineButton, MediaData, MediaType, OutboundMedia, ReplyAbortReason,
    ReplyStreamError, ReplyStreamErrorKind, ReplyStreamFrame, ReplyStreamPhase, ReplyStreamTarget,
    RichReply, RichReplyReceipt,
};
use serde_json::{json, Value};
use uuid::Uuid;

use super::api::{RichUpload, TelegramBotApi};
use super::rich::{
    compile_markdown, compile_plain_text, RichFormat, RichSegment, ThinkingLabel,
    RICH_MESSAGE_MAX_BLOCKS, RICH_MESSAGE_MAX_CHARS,
};
use crate::channel::media_helpers::materialize_to_bytes;

const TELEGRAM_RICH_MAX_MEDIA: usize = 50;
const TELEGRAM_PHOTO_MAX_BYTES: usize = 10 * 1024 * 1024;
const TELEGRAM_OTHER_MEDIA_MAX_BYTES: usize = 50 * 1024 * 1024;
const TELEGRAM_LOCAL_MEDIA_TOTAL_MAX_BYTES: usize = 100 * 1024 * 1024;
const TELEGRAM_BUTTON_TEXT_MAX_CHARS: usize = 64;
const TELEGRAM_BUTTON_CALLBACK_MAX_BYTES: usize = 64;
const TELEGRAM_FILENAME_MAX_CHARS: usize = 128;
const TELEGRAM_RICH_SEGMENT_INTERVAL: Duration = Duration::from_millis(1_050);

#[derive(Clone, Debug, PartialEq, Eq)]
struct TelegramTarget {
    chat_id: i64,
    thread_id: Option<i64>,
    reply_to_message_id: Option<i64>,
}

impl TelegramTarget {
    fn preview(target: &ReplyStreamTarget) -> Result<Self, ReplyStreamError> {
        if !matches!(target.chat_type, ChatType::Dm) {
            return Err(ReplyStreamError::new(
                ReplyStreamErrorKind::Unsupported,
                "Telegram rich drafts are supported only in private chats",
            ));
        }
        Self::parse(target)
    }

    fn final_reply(target: &ReplyStreamTarget) -> Result<Self, ReplyStreamError> {
        if !matches!(
            target.chat_type,
            ChatType::Dm | ChatType::Group | ChatType::Forum
        ) {
            return Err(ReplyStreamError::new(
                ReplyStreamErrorKind::Unsupported,
                "Telegram rich replies do not support this chat type",
            ));
        }
        Self::parse(target)
    }

    fn parse(target: &ReplyStreamTarget) -> Result<Self, ReplyStreamError> {
        let chat_id = parse_nonzero_i64("chat_id", &target.chat_id)?;
        let thread_id = parse_optional_positive_i64("thread_id", target.thread_id.as_deref())?;
        let reply_to_message_id = parse_optional_positive_i64(
            "reply_to_message_id",
            target.reply_to_message_id.as_deref(),
        )?;
        Ok(Self {
            chat_id,
            thread_id,
            reply_to_message_id,
        })
    }
}

fn parse_nonzero_i64(field: &str, raw: &str) -> Result<i64, ReplyStreamError> {
    let value = raw.parse::<i64>().map_err(|_| {
        ReplyStreamError::new(
            ReplyStreamErrorKind::InvalidTarget,
            format!("Telegram {field} must be a valid integer"),
        )
    })?;
    if value == 0 {
        return Err(ReplyStreamError::new(
            ReplyStreamErrorKind::InvalidTarget,
            format!("Telegram {field} must be non-zero"),
        ));
    }
    Ok(value)
}

fn parse_optional_positive_i64(
    field: &str,
    raw: Option<&str>,
) -> Result<Option<i64>, ReplyStreamError> {
    raw.map(|value| {
        let value = value.parse::<i64>().map_err(|_| {
            ReplyStreamError::new(
                ReplyStreamErrorKind::InvalidTarget,
                format!("Telegram {field} must be a valid integer"),
            )
        })?;
        if value <= 0 {
            return Err(ReplyStreamError::new(
                ReplyStreamErrorKind::InvalidTarget,
                format!("Telegram {field} must be positive"),
            ));
        }
        Ok(value)
    })
    .transpose()
}

fn rich_format(api: &TelegramBotApi) -> RichFormat {
    if api.prefers_rich_blocks() {
        RichFormat::Blocks
    } else {
        RichFormat::Html
    }
}

fn thinking_label(phase: ReplyStreamPhase) -> ThinkingLabel {
    match phase {
        ReplyStreamPhase::Generating => ThinkingLabel::Generating,
        ReplyStreamPhase::RunningTools => ThinkingLabel::RunningTools,
        ReplyStreamPhase::Finalizing => ThinkingLabel::Finalizing,
    }
}

fn compile_segments(markdown: &str) -> Result<Vec<RichSegment>, ReplyStreamError> {
    Ok(match compile_markdown(markdown) {
        Ok(segments) => segments,
        Err(error) => {
            app_warn!(
                "channel",
                "telegram",
                "Rich Markdown structure rejected; using escaped plain rich segments: {}",
                error
            );
            compile_plain_text(markdown)
        }
    })
}

fn compile_latest_draft_segment(markdown: &str) -> Result<RichSegment, ReplyStreamError> {
    let segments = compile_segments(markdown)?;
    segments.into_iter().last().ok_or_else(|| {
        ReplyStreamError::new(
            ReplyStreamErrorKind::InvalidContent,
            "Telegram rich draft did not contain a renderable segment",
        )
    })
}

fn new_draft_id() -> i64 {
    let bytes = Uuid::new_v4().into_bytes();
    let mut high = [0_u8; 8];
    high.copy_from_slice(&bytes[..8]);
    let candidate = i64::from_be_bytes(high) & i64::MAX;
    candidate.max(1)
}

fn draft_body(
    target: &TelegramTarget,
    draft_id: i64,
    segment: &RichSegment,
    format: RichFormat,
    phase: ReplyStreamPhase,
) -> Value {
    let mut body = json!({
        "chat_id": target.chat_id,
        "draft_id": draft_id,
        "rich_message": segment.input_message(format, Some(thinking_label(phase))),
    });
    if let Some(thread_id) = target.thread_id {
        body["message_thread_id"] = Value::from(thread_id);
    }
    body
}

async fn send_draft_snapshot(
    api: &TelegramBotApi,
    target: &TelegramTarget,
    draft_id: i64,
    markdown: &str,
    phase: ReplyStreamPhase,
    format: RichFormat,
) -> Result<(), ReplyStreamError> {
    // The preview is temporary UI: show the newest bounded segment while the
    // independent final path still sends every segment in original order.
    let segment = compile_latest_draft_segment(markdown)?;
    let acknowledged = api
        .send_rich_message_draft(draft_body(target, draft_id, &segment, format, phase))
        .await
        .map_err(|error| error.into_reply_stream_error())?;
    if !acknowledged {
        return Err(ReplyStreamError::new(
            ReplyStreamErrorKind::Rejected,
            "Telegram rejected the rich draft without acknowledging it",
        ));
    }
    Ok(())
}

struct TelegramReplyStream {
    api: Arc<TelegramBotApi>,
    target: TelegramTarget,
    draft_id: i64,
    revision: u64,
    format: RichFormat,
}

#[async_trait]
impl ChannelReplyStream for TelegramReplyStream {
    async fn push(&mut self, frame: &ReplyStreamFrame) -> Result<(), ReplyStreamError> {
        if frame.revision <= self.revision {
            return Err(ReplyStreamError::new(
                ReplyStreamErrorKind::InvalidContent,
                "Telegram rich draft revision must increase monotonically",
            ));
        }
        send_draft_snapshot(
            &self.api,
            &self.target,
            self.draft_id,
            &frame.markdown_snapshot,
            frame.phase,
            self.format,
        )
        .await?;
        self.revision = frame.revision;
        Ok(())
    }

    async fn commit(
        self: Box<Self>,
        final_reply: &RichReply,
    ) -> Result<RichReplyReceipt, ReplyStreamError> {
        // Draft state is intentionally irrelevant here. Telegram requires an
        // independent sendRichMessage call to persist the final reply.
        deliver_final(&self.api, &self.target, final_reply, self.format).await
    }

    async fn fail(self: Box<Self>, error_text: &str) -> Result<(), ReplyStreamError> {
        let reply = RichReply {
            markdown: if error_text.trim().is_empty() {
                "⚠️ Reply generation failed.".to_string()
            } else {
                error_text.to_string()
            },
            media: Vec::new(),
            buttons: Vec::new(),
        };
        deliver_final(&self.api, &self.target, &reply, self.format)
            .await
            .map(|_| ())
    }

    async fn abort(self: Box<Self>, _reason: ReplyAbortReason) -> Result<(), ReplyStreamError> {
        // Rich drafts have no delete/cancel endpoint and expire after 30s.
        Ok(())
    }
}

pub(super) async fn open_reply_stream(
    api: Arc<TelegramBotApi>,
    target: &ReplyStreamTarget,
    first: &ReplyStreamFrame,
) -> Result<Box<dyn ChannelReplyStream>, ReplyStreamError> {
    let target = TelegramTarget::preview(target)?;
    let draft_id = new_draft_id();
    let format = rich_format(&api);
    send_draft_snapshot(
        &api,
        &target,
        draft_id,
        &first.markdown_snapshot,
        first.phase,
        format,
    )
    .await?;
    Ok(Box::new(TelegramReplyStream {
        api,
        target,
        draft_id,
        revision: first.revision,
        format,
    }))
}

pub(super) async fn send_rich_reply(
    api: Arc<TelegramBotApi>,
    target: &ReplyStreamTarget,
    reply: &RichReply,
) -> Result<RichReplyReceipt, ReplyStreamError> {
    let target = TelegramTarget::final_reply(target)?;
    let format = rich_format(&api);
    deliver_final(&api, &target, reply, format).await
}

#[derive(Debug)]
struct PreparedMedia {
    offered_index: usize,
    block: Value,
    caption_chars: usize,
    upload: Option<RichUpload>,
}

fn media_shape(media_type: &MediaType) -> Option<(&'static str, &'static str)> {
    match media_type {
        MediaType::Photo => Some(("photo", "photo")),
        MediaType::Video => Some(("video", "video")),
        MediaType::Audio => Some(("audio", "audio")),
        MediaType::Voice => Some(("voice_note", "voice_note")),
        MediaType::Animation => Some(("animation", "animation")),
        MediaType::Document | MediaType::Sticker => None,
    }
}

fn media_size_limit(media_type: &MediaType) -> usize {
    if matches!(media_type, MediaType::Photo) {
        TELEGRAM_PHOTO_MAX_BYTES
    } else {
        TELEGRAM_OTHER_MEDIA_MAX_BYTES
    }
}

fn safe_upload_filename(raw: &str, index: usize, media_type: &MediaType) -> String {
    let basename = Path::new(raw)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(raw);
    let candidate = basename
        .chars()
        .filter(|character| !character.is_control() && !matches!(character, '/' | '\\' | ':' | '"'))
        .take(TELEGRAM_FILENAME_MAX_CHARS)
        .collect::<String>();
    let candidate = candidate.trim();
    if candidate.is_empty() {
        let extension = match media_type {
            MediaType::Photo => "jpg",
            MediaType::Video => "mp4",
            MediaType::Audio => "mp3",
            MediaType::Voice => "ogg",
            MediaType::Animation => "gif",
            MediaType::Document | MediaType::Sticker => "bin",
        };
        format!("telegram-media-{index}.{extension}")
    } else {
        candidate.to_string()
    }
}

fn safe_mime_type(raw: &str) -> Option<String> {
    let (top, subtype) = raw.split_once('/')?;
    let token = |value: &str| {
        !value.is_empty()
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(
                        byte,
                        b'!' | b'#' | b'$' | b'&' | b'^' | b'_' | b'.' | b'+' | b'-'
                    )
            })
    };
    (token(top) && token(subtype)).then(|| raw.to_ascii_lowercase())
}

async fn checked_public_media_url(raw: &str) -> Option<String> {
    let parsed = url::Url::parse(raw).ok()?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return None;
    }
    let config = ha_core::config::cached_config();
    let policy = config.ssrf.default_policy;
    let trusted_hosts = config.ssrf.trusted_hosts.clone();
    drop(config);
    ha_core::security::ssrf::check_url(parsed.as_str(), policy, &trusted_hosts)
        .await
        .ok()
        .map(|url| url.to_string())
}

fn media_block(media_type: &MediaType, source: String, caption: Option<&str>) -> Option<Value> {
    let (block_type, field) = media_shape(media_type)?;
    let mut block = json!({ "type": block_type });
    block[field] = json!({
        "type": block_type,
        "media": source,
    });
    if let Some(caption) = caption.filter(|caption| !caption.is_empty()) {
        block["caption"] = json!({ "text": caption });
    }
    Some(block)
}

async fn prepare_media(media: &[OutboundMedia]) -> Vec<PreparedMedia> {
    let mut prepared = Vec::new();
    let mut local_bytes = 0usize;

    for (offered_index, item) in media.iter().enumerate() {
        if prepared.len() == TELEGRAM_RICH_MAX_MEDIA {
            break;
        }
        if media_shape(&item.media_type).is_none() {
            break;
        }
        let caption_chars = item
            .caption
            .as_deref()
            .map(str::chars)
            .map(Iterator::count)
            .unwrap_or(0);
        if caption_chars > RICH_MESSAGE_MAX_CHARS {
            break;
        }

        let (source, upload) = match &item.data {
            MediaData::Url(url) => {
                let Some(url) = checked_public_media_url(url).await else {
                    app_warn!(
                        "channel",
                        "telegram",
                        "Telegram rich final left media item {} on the legacy lane after URL preflight",
                        offered_index
                    );
                    break;
                };
                (url, None)
            }
            MediaData::FilePath(_) | MediaData::Bytes(_) => {
                let remaining = TELEGRAM_LOCAL_MEDIA_TOTAL_MAX_BYTES.saturating_sub(local_bytes);
                if remaining == 0 {
                    break;
                }
                let max_bytes = media_size_limit(&item.media_type).min(remaining);
                let Ok(materialized) =
                    materialize_to_bytes(&item.data, &item.media_type, max_bytes).await
                else {
                    app_warn!(
                        "channel",
                        "telegram",
                        "Telegram rich final left media item {} on the legacy lane after upload preflight",
                        offered_index
                    );
                    break;
                };
                if materialized.bytes.is_empty() {
                    break;
                }
                local_bytes = local_bytes.saturating_add(materialized.bytes.len());
                let name = format!("media_{offered_index}");
                let source = format!("attach://{name}");
                let upload = RichUpload {
                    name,
                    filename: safe_upload_filename(
                        &materialized.filename,
                        offered_index,
                        &item.media_type,
                    ),
                    mime_type: safe_mime_type(&materialized.mime),
                    bytes: materialized.bytes,
                };
                (source, Some(upload))
            }
        };

        let Some(block) = media_block(&item.media_type, source, item.caption.as_deref()) else {
            break;
        };
        prepared.push(PreparedMedia {
            offered_index,
            block,
            caption_chars,
            upload,
        });
    }
    prepared
}

enum PlannedContent {
    Blocks(Vec<Value>),
    Html(String),
}

struct PlannedPart {
    content: PlannedContent,
    text_chars: usize,
    block_count: usize,
    media_count: usize,
    uploads: Vec<RichUpload>,
    consumed_media: Vec<usize>,
}

impl PlannedPart {
    fn from_segment(segment: RichSegment, format: RichFormat) -> Option<Self> {
        let text_chars = segment.text_chars();
        let block_count = segment.block_count();
        let content = match format {
            RichFormat::Blocks if !segment.blocks.is_empty() => {
                PlannedContent::Blocks(segment.blocks)
            }
            RichFormat::Html if !segment.html.is_empty() => PlannedContent::Html(segment.html),
            RichFormat::Blocks | RichFormat::Html => return None,
        };
        Some(Self {
            content,
            text_chars,
            block_count,
            media_count: 0,
            uploads: Vec::new(),
            consumed_media: Vec::new(),
        })
    }

    fn empty_blocks() -> Self {
        Self {
            content: PlannedContent::Blocks(Vec::new()),
            text_chars: 0,
            block_count: 0,
            media_count: 0,
            uploads: Vec::new(),
            consumed_media: Vec::new(),
        }
    }

    fn can_append_media(&self, media: &PreparedMedia) -> bool {
        matches!(self.content, PlannedContent::Blocks(_))
            && self.block_count < RICH_MESSAGE_MAX_BLOCKS
            && self.media_count < TELEGRAM_RICH_MAX_MEDIA
            && self.text_chars.saturating_add(media.caption_chars) <= RICH_MESSAGE_MAX_CHARS
    }

    fn append_media(&mut self, media: PreparedMedia) {
        let PlannedContent::Blocks(blocks) = &mut self.content else {
            unreachable!("media is appended only to a block rich message");
        };
        blocks.push(media.block);
        self.block_count += 1;
        self.text_chars = self.text_chars.saturating_add(media.caption_chars);
        self.media_count += 1;
        if let Some(upload) = media.upload {
            self.uploads.push(upload);
        }
        self.consumed_media.push(media.offered_index);
    }

    fn into_rich_message(self) -> (Value, Vec<RichUpload>, Vec<usize>) {
        let rich_message = match self.content {
            PlannedContent::Blocks(blocks) => json!({
                "blocks": blocks,
                "skip_entity_detection": true,
            }),
            PlannedContent::Html(html) => json!({
                "html": html,
                "skip_entity_detection": true,
            }),
        };
        (rich_message, self.uploads, self.consumed_media)
    }
}

struct FinalRequest {
    body: Value,
    uploads: Vec<RichUpload>,
    consumed_media: Vec<usize>,
}

fn build_inline_keyboard(buttons: &[Vec<InlineButton>]) -> Result<Option<Value>, ReplyStreamError> {
    let mut rows = Vec::new();
    for row in buttons {
        if row.is_empty() {
            return Err(ReplyStreamError::new(
                ReplyStreamErrorKind::InvalidContent,
                "Telegram inline keyboards cannot contain empty rows",
            ));
        }
        let mut output_row = Vec::new();
        for button in row {
            let text_chars = button.text.chars().count();
            if text_chars == 0
                || text_chars > TELEGRAM_BUTTON_TEXT_MAX_CHARS
                || button.text.chars().any(char::is_control)
            {
                return Err(ReplyStreamError::new(
                    ReplyStreamErrorKind::InvalidContent,
                    "Telegram button text must contain 1 to 64 visible characters",
                ));
            }
            if button.url.is_some() && button.callback_data.is_some() {
                return Err(ReplyStreamError::new(
                    ReplyStreamErrorKind::InvalidContent,
                    "Telegram buttons cannot contain both URL and callback actions",
                ));
            }
            let action = if let Some(raw_url) = button.url.as_deref() {
                let parsed = url::Url::parse(raw_url).map_err(|_| {
                    ReplyStreamError::new(
                        ReplyStreamErrorKind::InvalidContent,
                        "Telegram button URL is invalid",
                    )
                })?;
                if !matches!(parsed.scheme(), "http" | "https" | "tg")
                    || parsed.host_str().is_none()
                    || !parsed.username().is_empty()
                    || parsed.password().is_some()
                {
                    return Err(ReplyStreamError::new(
                        ReplyStreamErrorKind::InvalidContent,
                        "Telegram button URL must use http, https, or tg without credentials",
                    ));
                }
                json!({ "text": button.text, "url": parsed.as_str() })
            } else {
                // InlineButton's common contract defines text as the explicit
                // callback fallback when callback_data is absent.
                let callback = button.callback_id();
                if callback.is_empty()
                    || callback.len() > TELEGRAM_BUTTON_CALLBACK_MAX_BYTES
                    || callback.chars().any(char::is_control)
                {
                    return Err(ReplyStreamError::new(
                        ReplyStreamErrorKind::InvalidContent,
                        "Telegram callback data must contain 1 to 64 UTF-8 bytes",
                    ));
                }
                json!({ "text": button.text, "callback_data": callback })
            };
            output_row.push(action);
        }
        if !output_row.is_empty() {
            rows.push(Value::Array(output_row));
        }
    }
    Ok((!rows.is_empty()).then(|| json!({ "inline_keyboard": rows })))
}

pub(super) fn validate_reply_buttons(
    buttons: &[Vec<InlineButton>],
) -> Result<(), ReplyStreamError> {
    build_inline_keyboard(buttons).map(|_| ())
}

fn plan_final_requests(
    target: &TelegramTarget,
    segments: Vec<RichSegment>,
    format: RichFormat,
    media: Vec<PreparedMedia>,
    buttons: &[Vec<InlineButton>],
) -> Result<Vec<FinalRequest>, ReplyStreamError> {
    if !media
        .iter()
        .enumerate()
        .all(|(expected, item)| item.offered_index == expected)
    {
        return Err(ReplyStreamError::new(
            ReplyStreamErrorKind::InvalidContent,
            "Telegram rich media must be a contiguous prefix of the offered items",
        ));
    }
    let mut parts = segments
        .into_iter()
        .filter_map(|segment| PlannedPart::from_segment(segment, format))
        .collect::<Vec<_>>();

    if matches!(format, RichFormat::Blocks) {
        for item in media {
            if parts
                .last()
                .is_none_or(|part| !part.can_append_media(&item))
            {
                parts.push(PlannedPart::empty_blocks());
            }
            parts
                .last_mut()
                .expect("media planner always has a destination")
                .append_media(item);
        }
    }

    if parts.is_empty() {
        return Err(ReplyStreamError::new(
            ReplyStreamErrorKind::InvalidContent,
            "Telegram rich reply has no persistable text or media",
        ));
    }

    // Validate the only terminal-only UI before any sendRichMessage mutation.
    let mut keyboard = build_inline_keyboard(buttons)?;
    let part_count = parts.len();
    let mut requests = Vec::with_capacity(part_count);
    for (index, part) in parts.into_iter().enumerate() {
        let (rich_message, uploads, consumed_media) = part.into_rich_message();
        let mut body = json!({
            "chat_id": target.chat_id,
            "rich_message": rich_message,
        });
        if let Some(thread_id) = target.thread_id {
            body["message_thread_id"] = Value::from(thread_id);
        }
        if index == 0 {
            if let Some(message_id) = target.reply_to_message_id {
                body["reply_parameters"] = json!({ "message_id": message_id });
            }
        }
        if index + 1 == part_count {
            if let Some(reply_markup) = keyboard.take() {
                body["reply_markup"] = reply_markup;
            }
        }
        requests.push(FinalRequest {
            body,
            uploads,
            consumed_media,
        });
    }
    Ok(requests)
}

fn ambiguous_after_partial_delivery(error: ReplyStreamError) -> ReplyStreamError {
    ReplyStreamError::new(
        ReplyStreamErrorKind::Ambiguous,
        format!(
            "Telegram accepted an earlier rich-message segment; retry is unsafe: {}",
            ha_core::logging::redact_sensitive(&error.to_string())
        ),
    )
}

async fn deliver_final(
    api: &TelegramBotApi,
    target: &TelegramTarget,
    reply: &RichReply,
    format: RichFormat,
) -> Result<RichReplyReceipt, ReplyStreamError> {
    // Compile and validate every local concern before the first non-idempotent
    // request. Custom roots stay on Rich HTML and leave media to the worker's
    // legacy lane because their Bot API version cannot be discovered safely.
    let segments = compile_segments(&reply.markdown)?;
    let media = if matches!(format, RichFormat::Blocks) {
        prepare_media(&reply.media).await
    } else {
        Vec::new()
    };
    let requests = plan_final_requests(target, segments, format, media, &reply.buttons)?;

    let mut accepted_any = false;
    let mut last_message_id = None;
    let mut consumed_media = Vec::new();
    for (index, request) in requests.into_iter().enumerate() {
        if index > 0 {
            // Bot API recommends avoiding more than roughly one message per
            // second in a single chat. Native rich sends bypass teloxide's
            // throttle layer, so pace multi-segment finals locally.
            tokio::time::sleep(TELEGRAM_RICH_SEGMENT_INTERVAL).await;
        }
        match api.send_rich_message(request.body, request.uploads).await {
            Ok(message) => {
                accepted_any = true;
                last_message_id = Some(message.message_id);
                consumed_media.extend(request.consumed_media);
            }
            Err(error) => {
                let error = error.into_reply_stream_error();
                return Err(if accepted_any {
                    ambiguous_after_partial_delivery(error)
                } else {
                    error
                });
            }
        }
    }

    let message_id = last_message_id.ok_or_else(|| {
        ReplyStreamError::new(
            ReplyStreamErrorKind::Ambiguous,
            "Telegram rich reply completed without a message identifier",
        )
    })?;
    Ok(RichReplyReceipt {
        message_id: message_id.to_string(),
        consumed_media,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(chat_type: ChatType) -> ReplyStreamTarget {
        ReplyStreamTarget {
            account_id: "telegram".into(),
            chat_id: "-100123".into(),
            chat_type,
            thread_id: Some("42".into()),
            reply_to_message_id: Some("7".into()),
            recipient_user_id: None,
            recipient_tenant_id: None,
        }
    }

    #[test]
    fn draft_body_contains_only_draft_coordinates_and_rich_snapshot() {
        let parsed = TelegramTarget::preview(&target(ChatType::Dm)).unwrap();
        let segment = compile_segments("**hello**").unwrap().remove(0);
        let body = draft_body(
            &parsed,
            9,
            &segment,
            RichFormat::Blocks,
            ReplyStreamPhase::Generating,
        );
        assert_eq!(body["chat_id"], -100123);
        assert_eq!(body["message_thread_id"], 42);
        assert_eq!(body["draft_id"], 9);
        assert!(body.get("reply_parameters").is_none());
        assert!(body.get("reply_markup").is_none());
        assert_eq!(body["rich_message"]["blocks"][0]["type"], "thinking");
    }

    #[test]
    fn targets_are_strict_and_drafts_are_dm_only() {
        assert_eq!(
            TelegramTarget::preview(&target(ChatType::Group))
                .unwrap_err()
                .kind,
            ReplyStreamErrorKind::Unsupported
        );
        let mut invalid = target(ChatType::Dm);
        invalid.thread_id = Some("not-an-id".into());
        assert_eq!(
            TelegramTarget::preview(&invalid).unwrap_err().kind,
            ReplyStreamErrorKind::InvalidTarget
        );
        for _ in 0..32 {
            assert_ne!(new_draft_id(), 0);
        }
    }

    #[test]
    fn buttons_preserve_callbacks_and_reject_ambiguous_actions() {
        let buttons = vec![vec![InlineButton {
            text: "Approve".into(),
            callback_data: Some("approval:abc:allow_once".into()),
            url: None,
        }]];
        let keyboard = build_inline_keyboard(&buttons).unwrap().unwrap();
        assert_eq!(
            keyboard["inline_keyboard"][0][0]["callback_data"],
            "approval:abc:allow_once"
        );

        let both = vec![vec![InlineButton {
            text: "Open".into(),
            callback_data: Some("open".into()),
            url: Some("https://example.com".into()),
        }]];
        assert_eq!(
            build_inline_keyboard(&both).unwrap_err().kind,
            ReplyStreamErrorKind::InvalidContent
        );
        assert_eq!(
            build_inline_keyboard(&[Vec::new()]).unwrap_err().kind,
            ReplyStreamErrorKind::InvalidContent
        );
    }

    #[test]
    fn final_plan_keeps_reply_first_thread_everywhere_and_keyboard_last() {
        let parsed = TelegramTarget::final_reply(&target(ChatType::Forum)).unwrap();
        let markdown = format!("first\n\n{}", "x".repeat(RICH_MESSAGE_MAX_CHARS));
        let segments = compile_segments(&markdown).unwrap();
        assert!(segments.len() >= 2);
        let buttons = vec![vec![InlineButton {
            text: "Continue".into(),
            callback_data: Some("continue".into()),
            url: None,
        }]];
        let requests =
            plan_final_requests(&parsed, segments, RichFormat::Blocks, Vec::new(), &buttons)
                .unwrap();
        assert!(requests.len() >= 2);
        assert!(requests[0].body.get("reply_parameters").is_some());
        assert!(requests[0].body.get("reply_markup").is_none());
        for request in &requests {
            assert_eq!(request.body["message_thread_id"], 42);
        }
        assert!(requests.last().unwrap().body.get("reply_markup").is_some());
    }

    #[test]
    fn draft_renders_latest_bounded_segment_from_full_canonical_snapshot() {
        let prefix = "a".repeat(RICH_MESSAGE_MAX_CHARS);
        let accumulated = format!("{prefix}\n\nlatest segment");
        let segment = compile_latest_draft_segment(&accumulated).unwrap();
        assert_eq!(segment.text_chars(), "latest segment".chars().count());
        assert!(segment.html.contains("latest segment"));
        assert!(!segment.html.contains(&"a".repeat(128)));

        let oversized_paragraph = format!("{}TAIL", "x".repeat(RICH_MESSAGE_MAX_CHARS));
        let rolling = compile_latest_draft_segment(&oversized_paragraph).unwrap();
        assert!(rolling.text_chars() <= RICH_MESSAGE_MAX_CHARS);
        assert!(rolling.html.ends_with("TAIL</p>"));
    }

    #[test]
    fn media_block_uses_typed_voice_note_and_caption() {
        let block = media_block(
            &MediaType::Voice,
            "attach://media_0".into(),
            Some("voice caption"),
        )
        .unwrap();
        assert_eq!(block["type"], "voice_note");
        assert_eq!(block["voice_note"]["type"], "voice_note");
        assert_eq!(block["voice_note"]["media"], "attach://media_0");
        assert_eq!(block["caption"]["text"], "voice caption");
    }

    #[test]
    fn final_plan_reports_only_media_embedded_in_the_rich_request() {
        let parsed = TelegramTarget::final_reply(&target(ChatType::Dm)).unwrap();
        let segments = compile_segments("result").unwrap();
        let photo = PreparedMedia {
            offered_index: 0,
            block: media_block(
                &MediaType::Photo,
                "https://example.com/image.png".into(),
                None,
            )
            .unwrap(),
            caption_chars: 0,
            upload: None,
        };
        let requests =
            plan_final_requests(&parsed, segments, RichFormat::Blocks, vec![photo], &[]).unwrap();

        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].consumed_media, vec![0]);
        assert_eq!(
            requests[0].body["rich_message"]["blocks"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()["type"],
            "photo"
        );
    }

    #[test]
    fn final_plan_rejects_a_non_prefix_media_receipt_shape_before_sending() {
        let parsed = TelegramTarget::final_reply(&target(ChatType::Dm)).unwrap();
        let segments = compile_segments("result").unwrap();
        let skipped_first_item = PreparedMedia {
            offered_index: 1,
            block: media_block(
                &MediaType::Photo,
                "https://example.com/image.png".into(),
                None,
            )
            .unwrap(),
            caption_chars: 0,
            upload: None,
        };

        let error = match plan_final_requests(
            &parsed,
            segments,
            RichFormat::Blocks,
            vec![skipped_first_item],
            &[],
        ) {
            Err(error) => error,
            Ok(_) => panic!("non-prefix native media must fail before a mutation"),
        };

        assert_eq!(error.kind, ReplyStreamErrorKind::InvalidContent);
    }
}
