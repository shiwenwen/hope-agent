/**
 * Escape HTML special characters then restore `<mark>`/`</mark>` tags only.
 *
 * The backend emits FTS5 `snippet()` output containing `<mark>...</mark>`
 * around matched terms. To prevent XSS from user-authored message content:
 *   1. Escape everything with an HTML entity pass.
 *   2. Turn the now-escaped `&lt;mark&gt;` / `&lt;/mark&gt;` back into raw
 *      tags (the only whitelisted tags).
 */
export function renderHighlightedSnippet(raw: string): string {
  const escaped = raw
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;")
  return escaped
    .replace(/&lt;mark&gt;/g, '<mark class="bg-primary/30 text-foreground rounded px-0.5">')
    .replace(/&lt;\/mark&gt;/g, "</mark>")
}
