/**
 * Types for the chat-input `@` file/folder mention feature.
 *
 * The popper triggers when the user types `@` in the chat textarea while a
 * working directory is set on the session. The token after `@` selects one of
 * three modes:
 * - empty token  → list working_dir top level
 * - contains `/` → "path mode": list the named subdirectory
 * - non-empty no `/` → "search mode": fuzzy search across working_dir
 *
 * The mirror overlay (chip rendering) consumes the same parser via
 * `parseMentions()` so the visual state stays in sync with the text state.
 */

import type { DirEntry, FileMatch } from "@/lib/transport";

/** Which data source is currently feeding the popper. */
export type MentionMode = "list" | "search";

/** A single popper row. Folded across both modes via {@link toMentionEntry}. */
export interface MentionEntry {
  name: string;
  /** Absolute path. */
  path: string;
  /** Path relative to working_dir (no leading `/`). */
  relPath: string;
  isDir: boolean;
}

export function entryFromDir(workingDir: string, e: DirEntry): MentionEntry {
  const rel = relativeFromBase(workingDir, e.path);
  return {
    name: e.name,
    path: e.path,
    relPath: rel,
    isDir: e.isDir,
  };
}

export function entryFromMatch(m: FileMatch): MentionEntry {
  return {
    name: m.name,
    path: m.path,
    relPath: m.relPath,
    isDir: m.isDir,
  };
}

function stripTrailingSlash(p: string): string {
  return p.endsWith("/") ? p.slice(0, -1) : p;
}

export function relativeFromBase(base: string, target: string): string {
  const b = stripTrailingSlash(base);
  if (target === b) return "";
  if (target.startsWith(b + "/")) return target.slice(b.length + 1);
  return target;
}

/**
 * Join `base` with a `/`-separated relative segment. Pass-through when `sub`
 * is already absolute (Unix `/...` or Windows `C:\...` / `C:/...`) so callers
 * don't double-prefix when an entry path slips through as absolute.
 */
export function joinAbs(base: string, sub: string): string {
  if (!sub) return stripTrailingSlash(base);
  if (sub.startsWith("/") || /^[A-Za-z]:[\\/]/.test(sub)) return sub;
  return `${stripTrailingSlash(base)}/${sub.replace(/^\/+/, "")}`;
}

/** A segment produced by the mirror parser; used to lay out the chip overlay. */
export type MentionSegment =
  | { kind: "text"; text: string }
  | { kind: "mention"; raw: string; relPath: string };
