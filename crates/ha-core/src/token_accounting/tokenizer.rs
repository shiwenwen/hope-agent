use std::fmt;

use serde_json::Value;
use tiktoken_rs::{cl100k_base_singleton, o200k_base_singleton, CoreBPE};

use super::TokenizerId;

#[derive(Debug, Clone)]
pub struct TokenizerError(pub String);

impl fmt::Display for TokenizerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TokenizerError {}

pub trait SyncTextTokenizer: Send + Sync {
    fn id(&self) -> TokenizerId;
    fn revision(&self) -> &'static str;
    fn count_text(&self, text: &str) -> Result<u64, TokenizerError>;

    fn count_json(&self, value: &Value) -> Result<u64, TokenizerError> {
        let serialized = serde_json::to_string(value)
            .map_err(|error| TokenizerError(format!("serialize JSON for token count: {error}")))?;
        self.count_text(&serialized)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TiktokenTokenizer {
    id: TokenizerId,
}

impl TiktokenTokenizer {
    pub const fn new(id: TokenizerId) -> Self {
        Self { id }
    }

    fn bpe(self) -> &'static CoreBPE {
        match self.id {
            TokenizerId::O200kBase => o200k_base_singleton(),
            TokenizerId::Cl100kBase => cl100k_base_singleton(),
        }
    }
}

impl SyncTextTokenizer for TiktokenTokenizer {
    fn id(&self) -> TokenizerId {
        self.id
    }

    fn revision(&self) -> &'static str {
        match self.id {
            TokenizerId::O200kBase => "tiktoken-rs-0.12.0:o200k_base",
            TokenizerId::Cl100kBase => "tiktoken-rs-0.12.0:cl100k_base",
        }
    }

    fn count_text(&self, text: &str) -> Result<u64, TokenizerError> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.bpe().encode_with_special_tokens(text).len() as u64
        }))
        .map_err(|_| {
            TokenizerError(format!(
                "{} tokenizer initialization failed",
                self.id.as_str()
            ))
        })
    }
}
