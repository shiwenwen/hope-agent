/**
 * Mention token parser, shared between:
 * - {@link useFileMention} — caret-aware "what is the user typing right now"
 * - {@link MentionMirrorOverlay} — segments the whole input string for chip rendering
 * - {@link expandMentionsToAttachments} — collects all mentions before send
 *
 * Token grammar (simplified from Claude Code's HAS_AT_SYMBOL_RE; ASCII-only path
 * chars in v1):
 *   `@"path with space.md"`  — quoted form for paths containing whitespace
 *   `@some/path/file.md`     — bare form, terminated by whitespace
 *
 * Trigger boundary: the `@` must be at start of input, after whitespace, or
 * after another mention. This avoids matching email addresses (`a@b.com`).
 */

import type { MentionSegment } from "./types";

/**
 * Matches a complete mention as a substring. The leading boundary (start of
 * input or whitespace) is in group 1; only `@`-onward is the mention itself.
 *
 * - `@"..."`  — quoted path (no embedded `"`)
 * - `@token`  — bare path, terminated by whitespace
 */
const MENTION_RE_SOURCE = /(^|\s)@(?:"([^"]+)"|([^\s]+))/;

export interface ParsedMention {
  /** Starting index of the `@` character in `input`. */
  start: number;
  /** Exclusive end index (one past the last character of the mention). */
  end: number;
  /** The full raw mention substring including the `@` and any quotes. */
  raw: string;
  /** Relative path inside the mention (unquoted). */
  relPath: string;
}

export function parseMentions(input: string): ParsedMention[] {
  const out: ParsedMention[] = [];
  // Construct a fresh /g regex per call so concurrent callers (overlay re-render
  // racing with send-time expansion) can't trip over each other's lastIndex.
  const re = new RegExp(MENTION_RE_SOURCE.source, "g");
  for (const m of input.matchAll(re)) {
    const boundary = m[1] ?? "";
    const quoted = m[2];
    const bare = m[3];
    const start = (m.index ?? 0) + boundary.length;
    const end = (m.index ?? 0) + m[0].length;
    out.push({
      start,
      end,
      raw: input.slice(start, end),
      relPath: quoted ?? bare ?? "",
    });
  }
  return out;
}

/**
 * Split `input` into alternating text and mention segments. Used by the mirror
 * overlay to render chip backgrounds aligned to the textarea's character grid.
 */
export function segmentInput(input: string): MentionSegment[] {
  const mentions = parseMentions(input);
  if (mentions.length === 0) return [{ kind: "text", text: input }];

  const segments: MentionSegment[] = [];
  let cursor = 0;
  for (const m of mentions) {
    if (m.start > cursor) {
      segments.push({ kind: "text", text: input.slice(cursor, m.start) });
    }
    segments.push({ kind: "mention", raw: m.raw, relPath: m.relPath });
    cursor = m.end;
  }
  if (cursor < input.length) {
    segments.push({ kind: "text", text: input.slice(cursor) });
  }
  return segments;
}

/**
 * Partial mention being typed at the caret. `anchor` points at the `@`,
 * `caret` is the current caret position, `token` is the text between them.
 */
export interface ActiveMention {
  anchor: number;
  caret: number;
  token: string;
  /** `true` when token starts with `"` (user opened a quoted path). */
  quoted: boolean;
}

const PARTIAL_TOKEN_CHARS = /[^\s"]/;

export function detectActiveMention(input: string, caret: number): ActiveMention | null {
  if (caret < 1 || caret > input.length) return null;
  let i = caret - 1;
  while (i >= 0) {
    const c = input[i];
    if (c === "@") {
      // The `@` must sit at start-of-input or after whitespace — this is what
      // rules out `email@host` from triggering the popper.
      const prev = i > 0 ? input[i - 1] : "";
      if (i === 0 || /\s/.test(prev)) {
        const token = input.slice(i + 1, caret);
        const quoted = token.startsWith('"');
        if (!quoted && /\s/.test(token)) return null;
        return { anchor: i, caret, token, quoted };
      }
      return null;
    }
    if (!PARTIAL_TOKEN_CHARS.test(c)) return null;
    i--;
  }
  return null;
}

/**
 * Wrap a relative path for insertion into the textarea, quoting it when it
 * contains whitespace so {@link parseMentions} round-trips correctly.
 */
export function formatMentionInsertion(relPath: string): string {
  if (/\s/.test(relPath)) return `@"${relPath}"`;
  return `@${relPath}`;
}
