mod backend;
mod prompt;
mod trait_impl;

pub use backend::SqliteMemoryBackend;
#[allow(deprecated)]
pub use prompt::format_prompt_summary;
pub use prompt::{format_prompt_summary_v2, format_prompt_summary_v2_with_refs, PromptMemoryRef};
// Context Pack (memory/dreaming/context_pack.rs) renders LLM-derived claim
// content into the cache-stable prefix and must reuse the same prompt-injection
// filter as the SQLite memory section (red line: no bypass).
pub(crate) use prompt::sanitize_for_prompt;

// open_default is unused but kept for future convenience
