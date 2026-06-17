import { HighlightStyle } from "@codemirror/language"
import { tags as t } from "@lezer/highlight"

/**
 * Theme-adaptive syntax highlighting for the note editor.
 *
 * Replaces CodeMirror's built-in `defaultHighlightStyle`, whose token colors
 * (markdown markers `#164`, keywords `#708`, comments `#940`, meta `#404740`)
 * are tuned for a light background and become unreadable on the dark theme.
 *
 * Structural markdown tokens resolve through the app's `--color-*` CSS variables
 * (defined in `src/index.css`, overridden by the `.dark` class), so the editor
 * follows the global light / dark theme automatically with no JS theme tracking
 * — matching the convention used by the other `cm/` themes. Code-block tokens
 * (rarely tokenized in markdown notes) use mid-tone accent hues legible on both
 * backgrounds.
 */
export const noteHighlightStyle = HighlightStyle.define([
  // ── Markdown structure (theme-driven) ──────────────────────────────
  { tag: t.heading, color: "var(--color-foreground)", fontWeight: "700" },
  { tag: t.strong, fontWeight: "700" },
  { tag: t.emphasis, fontStyle: "italic" },
  { tag: t.strikethrough, textDecoration: "line-through" },
  { tag: t.monospace, color: "var(--color-foreground)" },
  { tag: t.quote, color: "var(--color-muted-foreground)", fontStyle: "italic" },
  // Markdown markers (`#`, `**`, `>`, list bullets, fence ```) + meta — dimmed.
  {
    tag: [t.processingInstruction, t.meta, t.contentSeparator],
    color: "var(--color-muted-foreground)",
  },
  // Links / URLs — indigo accent matching the wikilink chip color.
  { tag: [t.link, t.url], color: "#6366f1", textDecoration: "underline" },

  // ── Code-block tokens (mid-tone, readable on light + dark) ─────────
  { tag: t.keyword, color: "#a855f7" },
  { tag: [t.number, t.bool, t.atom], color: "#d97706" },
  { tag: [t.string, t.special(t.string)], color: "#16a34a" },
  { tag: [t.comment, t.lineComment, t.blockComment], color: "var(--color-muted-foreground)", fontStyle: "italic" },
  { tag: [t.typeName, t.className, t.tagName], color: "#0891b2" },
  { tag: [t.function(t.variableName), t.labelName], color: "#2563eb" },
  { tag: [t.propertyName, t.attributeName], color: "#0d9488" },
  { tag: t.invalid, color: "var(--color-destructive)" },
])
