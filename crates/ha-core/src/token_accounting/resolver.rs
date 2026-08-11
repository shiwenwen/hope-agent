use super::{ProviderFamily, TiktokenTokenizer, TokenizerId};

pub const TOKENIZER_REGISTRY_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy)]
pub struct ResolvedTokenizer {
    pub tokenizer: TiktokenTokenizer,
    pub registry_version: u32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TokenizerResolver;

impl TokenizerResolver {
    pub fn resolve(&self, provider: ProviderFamily, model: &str) -> Option<ResolvedTokenizer> {
        let model = canonical_model(model)?;
        let tokenizer_id = match provider {
            ProviderFamily::OpenAiChat
            | ProviderFamily::OpenAiResponses
            | ProviderFamily::Codex => openai_tokenizer(model)?,
            ProviderFamily::Anthropic | ProviderFamily::Unknown => return None,
        };
        Some(ResolvedTokenizer {
            tokenizer: TiktokenTokenizer::new(tokenizer_id),
            registry_version: TOKENIZER_REGISTRY_VERSION,
        })
    }
}

fn canonical_model(model: &str) -> Option<&str> {
    let model = model.trim();
    if model.is_empty() {
        return None;
    }
    // Compatible endpoints commonly prefix a namespace. Only the final path
    // segment is a model id; do not use substring matching.
    Some(model.rsplit('/').next().unwrap_or(model))
}

fn prefix_with_boundary(model: &str, prefix: &str) -> bool {
    model == prefix
        || model
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('-') || rest.starts_with('.'))
}

fn openai_tokenizer(model: &str) -> Option<TokenizerId> {
    if [
        "gpt-5",
        "gpt-4.1",
        "gpt-4o",
        "gpt-4.5",
        "o1",
        "o3",
        "o4",
        "codex-mini",
        "codex",
    ]
    .iter()
    .any(|prefix| prefix_with_boundary(model, prefix))
    {
        return Some(TokenizerId::O200kBase);
    }
    if [
        "gpt-4",
        "gpt-3.5-turbo",
        "text-embedding-3",
        "text-embedding-ada-002",
    ]
    .iter()
    .any(|prefix| prefix_with_boundary(model, prefix))
    {
        return Some(TokenizerId::Cl100kBase);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_requires_model_boundaries() {
        let resolver = TokenizerResolver;
        assert!(resolver
            .resolve(ProviderFamily::OpenAiResponses, "gpt-5.2-2026-01-01")
            .is_some());
        assert!(resolver
            .resolve(ProviderFamily::OpenAiResponses, "vendor/gpt-4o-mini")
            .is_some());
        assert!(resolver
            .resolve(ProviderFamily::OpenAiResponses, "gpt-5ish")
            .is_none());
        assert!(resolver
            .resolve(ProviderFamily::Anthropic, "gpt-5")
            .is_none());
    }
}
