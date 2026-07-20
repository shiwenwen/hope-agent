// GitHub-style heading slugs for the built-in user manual.
//
// MUST stay byte-identical to the Rust implementation
// (crates/ha-core/src/manual/model.rs `github_slug`): backend search anchors,
// intra-doc `#anchor` links and the heading `id`s injected at render time all
// have to agree. Contract locked by tests on both sides sharing the same
// ground-truth pairs taken from the real docs.

/** Rust `char::is_alphanumeric` ≈ Unicode Alphabetic ∪ Number. */
const ALPHANUMERIC = /[\p{Alphabetic}\p{N}]/u

export function manualSlug(text: string): string {
  let out = ""
  for (const c of text.trim()) {
    if (c === " ") {
      out += "-"
    } else if (c === "-" || c === "_") {
      out += c
    } else if (ALPHANUMERIC.test(c)) {
      out += c.toLowerCase()
    }
    // Everything else (punctuation, symbols, emoji) is dropped.
  }
  return out
}
