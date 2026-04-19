//! 飞书出站附件：物化字节 → upload_image / upload_file 拿 key → 发 image / file 消息。
//!
//! 飞书原生 image / file 消息**不携带 caption**。dispatcher 单独发同轮的 text chunk
//! 承接说明文字（[`crate::channel::worker::dispatcher::send_final_reply`]），所以
//! 这里把 `OutboundMedia.caption` 静默丢弃；如果将来有不走 dispatcher 直接构造
//! `OutboundMedia { caption: Some(_), .. }` 的调用方，需另行追发 text 消息。

use std::path::Path;

use anyhow::Result;

use crate::channel::media_helpers::materialize_to_bytes;
use crate::channel::types::{MediaType, OutboundMedia};

use super::api::FeishuApi;

/// 飞书图片单条上限官方文档为 10 MiB；文件 30 MiB。统一按较松的 30 MiB 收口
/// （Photo 实际更小但服务端会拒绝，错误透传给 dispatcher 即可）。
const MAX_FEISHU_FILE_BYTES: usize = 30 * 1024 * 1024;

pub async fn send_outbound_media(
    api: &FeishuApi,
    receive_id: &str,
    media: &OutboundMedia,
    reply_to: Option<&str>,
) -> Result<String> {
    let m = materialize_to_bytes(&media.data, &media.media_type, MAX_FEISHU_FILE_BYTES).await?;
    match media.media_type {
        MediaType::Photo => {
            let key = api
                .upload_image(m.bytes, &m.filename, &m.mime, "message")
                .await?;
            api.send_image_message(receive_id, &key, reply_to).await
        }
        _ => {
            let file_type = feishu_file_type(&media.media_type, &m.filename);
            let key = api
                .upload_file(m.bytes, &m.filename, &m.mime, file_type)
                .await?;
            api.send_file_message(receive_id, &key, reply_to).await
        }
    }
}

/// `MediaType` + 文件扩展名 → 飞书 `file_type`（opus / mp4 / pdf / doc / xls / ppt / stream）。
pub fn feishu_file_type(media_type: &MediaType, filename: &str) -> &'static str {
    match media_type {
        MediaType::Audio | MediaType::Voice => "opus",
        MediaType::Video | MediaType::Animation => "mp4",
        MediaType::Document => {
            let ext = Path::new(filename)
                .extension()
                .and_then(|s| s.to_str())
                .map(str::to_ascii_lowercase)
                .unwrap_or_default();
            match ext.as_str() {
                "pdf" => "pdf",
                "doc" | "docx" => "doc",
                "xls" | "xlsx" => "xls",
                "ppt" | "pptx" => "ppt",
                _ => "stream",
            }
        }
        // Photo 由 upload_image 单独处理；Sticker 飞书无原生支持，按 stream 兜底。
        MediaType::Photo | MediaType::Sticker => "stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_and_video_map_to_opus_and_mp4() {
        assert_eq!(feishu_file_type(&MediaType::Audio, "ignore.bin"), "opus");
        assert_eq!(feishu_file_type(&MediaType::Voice, "v.bin"), "opus");
        assert_eq!(feishu_file_type(&MediaType::Video, "any.txt"), "mp4");
        assert_eq!(feishu_file_type(&MediaType::Animation, "x.gif"), "mp4");
    }

    #[test]
    fn document_extension_mapping() {
        assert_eq!(feishu_file_type(&MediaType::Document, "report.pdf"), "pdf");
        assert_eq!(feishu_file_type(&MediaType::Document, "Memo.DOCX"), "doc");
        assert_eq!(feishu_file_type(&MediaType::Document, "data.xlsx"), "xls");
        assert_eq!(feishu_file_type(&MediaType::Document, "deck.pptx"), "ppt");
        assert_eq!(feishu_file_type(&MediaType::Document, "blob.bin"), "stream");
        assert_eq!(feishu_file_type(&MediaType::Document, "noext"), "stream");
    }
}
