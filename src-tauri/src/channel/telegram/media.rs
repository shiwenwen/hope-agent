use teloxide::types::{InputFile, PhotoSize};

use crate::channel::types::{InboundMedia, MediaType};

/// Convert a Telegram photo array to our InboundMedia (picks highest resolution).
pub fn photo_to_inbound(photos: &[PhotoSize]) -> Option<InboundMedia> {
    // Telegram sends multiple sizes; pick the largest
    let best = photos.iter().max_by_key(|p| p.width * p.height)?;
    Some(InboundMedia {
        media_type: MediaType::Photo,
        file_id: best.file.id.clone(),
        file_url: None,
        mime_type: Some("image/jpeg".to_string()),
        file_size: Some(best.file.size as u64),
        caption: None,
    })
}

/// Create an InputFile from a URL string.
pub fn input_file_from_url(url: &str) -> InputFile {
    InputFile::url(url.parse().expect("Invalid URL"))
}

/// Create an InputFile from a file path.
pub fn input_file_from_path(path: &str) -> InputFile {
    InputFile::file(std::path::PathBuf::from(path))
}

/// Create an InputFile from bytes.
pub fn input_file_from_bytes(data: Vec<u8>, filename: &str) -> InputFile {
    InputFile::memory(data).file_name(filename.to_string())
}

/// Map our MediaData to a teloxide InputFile.
pub fn media_data_to_input_file(data: &crate::channel::types::MediaData) -> InputFile {
    match data {
        crate::channel::types::MediaData::Url(url) => input_file_from_url(url),
        crate::channel::types::MediaData::FilePath(path) => input_file_from_path(path),
        crate::channel::types::MediaData::Bytes(bytes) => {
            input_file_from_bytes(bytes.clone(), "file")
        }
    }
}
