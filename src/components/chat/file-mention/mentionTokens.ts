/**
 * Mention token parser, shared between:
 * - {@link useFileMention} — caret-aware "what is the user typing right now"
 * - `MentionComposerInput` — segments the whole input string for chip rendering
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
// Bare token excludes `[` so a `@`-glued `[[note]]` (e.g. `@[[My Note]]`) is not
// parsed as a file mention at send time — `[[ ]]` belongs to the note picker.
const MENTION_RE_SOURCE = /(^|\s)@(?:"([^"]+)"|([^\s[]+))/;

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

/** Completed `[[note]]` references (closed `]]`), for the mirror overlay chips. */
const NOTE_RE_SOURCE = /\[\[([^\]\n]+)\]\]/;

export function parseNoteRefs(
  input: string,
): Array<{ start: number; end: number; raw: string }> {
  const out: Array<{ start: number; end: number; raw: string }> = [];
  const re = new RegExp(NOTE_RE_SOURCE.source, "g");
  for (const m of input.matchAll(re)) {
    const start = m.index ?? 0;
    out.push({ start, end: start + m[0].length, raw: m[0] });
  }
  return out;
}

/**
 * Split `input` into alternating text / file-mention / note-ref segments. Used
 * by the mirror overlay to render chip backgrounds aligned to the textarea's
 * character grid. `files` gates `@path` chips (working dir set); `notes` gates
 * `[[note]]` chips (note mention enabled). The two never overlap — the `@`
 * grammar excludes `[`.
 */
export function segmentInput(
  input: string,
  opts?: { files?: boolean; notes?: boolean },
): MentionSegment[] {
  const files = opts?.files ?? true;
  const notes = opts?.notes ?? false;

  const spans: Array<{ start: number; end: number; seg: MentionSegment }> = [];
  if (files) {
    for (const m of parseMentions(input)) {
      spans.push({
        start: m.start,
        end: m.end,
        seg: { kind: "mention", raw: m.raw, relPath: m.relPath },
      });
    }
  }
  if (notes) {
    for (const n of parseNoteRefs(input)) {
      spans.push({ start: n.start, end: n.end, seg: { kind: "note", raw: n.raw } });
    }
  }
  if (spans.length === 0) return [{ kind: "text", text: input }];

  spans.sort((a, b) => a.start - b.start);
  const segments: MentionSegment[] = [];
  let cursor = 0;
  for (const s of spans) {
    if (s.start < cursor) continue; // defensive: skip any overlap
    if (s.start > cursor) {
      segments.push({ kind: "text", text: input.slice(cursor, s.start) });
    }
    segments.push(s.seg);
    cursor = s.end;
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
    // Cede a `[[` opener to the note picker: if we hit `[[` before the `@`, this
    // is a `[[note]]` context, not a file mention — keep the two triggers disjoint.
    if (c === "[" && i > 0 && input[i - 1] === "[") return null;
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
