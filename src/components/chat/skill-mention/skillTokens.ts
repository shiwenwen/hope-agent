/**
 * Pure helpers for the chat composer's `@skill` mention — a curated, fixed
 * allowlist of built-in skills the user can activate from the `@` menu (office
 * trio + data analytics + browser + mac control). Parallel to
 * {@link parseMentions} (files) and the `[[note]]` picker, but the resolvable set is closed: the backend
 * (`skills::mention::resolve_inline_skill_mentions`) only activates the same
 * allowlist, so an arbitrary `#skill:foo` never injects anything.
 *
 * The token is a markdown link `[@<label>](#skill:<name>)` (see below). Its `@`
 * sits right after `[`, so it is structurally disjoint from the bare `@token`
 * file-mention grammar — no whitespace-boundary rule is needed, and `email`
 * text can't accidentally form one. Friendly labels live in i18n (keyed here)
 * and icons in `SkillMentionIcon`; the backend only ships `{ name, description }`.
 */

/** A row returned by the `list_mentionable_skills` RPC. */
export interface MentionableSkill {
  /** Canonical skill id — also the `@skill:<name>` token. */
  name: string
  /** Frontmatter blurb, shown as the menu-row tooltip. */
  description: string
}

/** Which glyph `SkillMentionIcon` renders for a catalog entry. */
export type SkillIconKind = "docx" | "pptx" | "xlsx" | "analytics" | "browser" | "mac"

export interface SkillMentionMeta {
  /** Full i18n key for the chip / menu label. */
  labelKey: string
  iconKind: SkillIconKind
  /** Menu grouping. */
  group: "office" | "analysis" | "control"
  /**
   * Lowercase match terms for the `@` filter, so typing `@excel` / `@word` /
   * `@浏览器` narrows to the right skill even though the canonical id doesn't
   * contain those words. The id itself is always matched too.
   */
  keywords: string[]
}

/**
 * The fixed, ordered catalog. Order is the menu display order. Membership here
 * is the frontend allowlist — a `@skill:<name>` only renders as a chip when
 * `name` is a key below (defense in depth alongside the backend allowlist).
 * `ha-mac-control` is macOS-only; the backend omits it from the menu off macOS,
 * so it simply never appears in the fetched rows on other platforms.
 */
export const SKILL_MENTION_CATALOG: Record<string, SkillMentionMeta> = {
  "office-docx": {
    labelKey: "chat.skillMention.labels.docx",
    iconKind: "docx",
    group: "office",
    keywords: ["docx", "doc", "word", "文档"],
  },
  "office-pptx": {
    labelKey: "chat.skillMention.labels.pptx",
    iconKind: "pptx",
    group: "office",
    keywords: ["pptx", "ppt", "powerpoint", "slide", "deck", "演示", "幻灯"],
  },
  "office-xlsx": {
    labelKey: "chat.skillMention.labels.xlsx",
    iconKind: "xlsx",
    group: "office",
    keywords: ["xlsx", "excel", "spreadsheet", "sheet", "csv", "表格"],
  },
  "ha-data-analytics": {
    labelKey: "chat.skillMention.labels.dataAnalytics",
    iconKind: "analytics",
    group: "analysis",
    keywords: [
      "data",
      "analytics",
      "analysis",
      "metric",
      "kpi",
      "dashboard",
      "report",
      "数据",
      "分析",
      "指标",
      "报告",
      "仪表盘",
    ],
  },
  "ha-browser": {
    labelKey: "chat.skillMention.labels.browser",
    iconKind: "browser",
    group: "control",
    keywords: ["browser", "web", "chrome", "浏览器"],
  },
  "ha-mac-control": {
    labelKey: "chat.skillMention.labels.mac",
    iconKind: "mac",
    group: "control",
    keywords: ["mac", "macos", "desktop", "control", "控制"],
  },
}

/** Allowlisted skill ids, in menu order. */
export const AT_MENTIONABLE_SKILL_NAMES = Object.keys(SKILL_MENTION_CATALOG)

export function isSkillMentionName(name: string): boolean {
  return Object.prototype.hasOwnProperty.call(SKILL_MENTION_CATALOG, name)
}

export function skillMentionMeta(name: string): SkillMentionMeta | undefined {
  return SKILL_MENTION_CATALOG[name]
}

/**
 * A skill mention is a markdown link `[@<label>](#skill:<name>)` (Codex-style:
 * friendly localized label as the visible text, stable id in the href). The
 * fragment href (`#skill:`) survives Streamdown's fixed rehype-sanitize schema
 * — a custom scheme like `skill://` would be stripped — so the **same token
 * renders as a chip in both the composer and the message history** (see
 * `MarkdownLink` in `common/MarkdownRenderer`). The backend reads only the id
 * from the href, so the label can be any localized text.
 */
export const SKILL_HREF_PREFIX = "#skill:"

/** A parsed `[@label](#skill:name)` token as a substring of the input. */
export interface ParsedSkillMention {
  /** Starting index of the opening `[`. */
  start: number
  /** Exclusive end index (one past the closing `)`). */
  end: number
  /** Full raw substring `[@label](#skill:name)`. */
  raw: string
  /** The skill id from the href. */
  name: string
  /** The visible label (link text, `@` stripped). */
  label: string
}

// `[@<label>](#skill:<name>)` — label is any non-`]` run; name is the skill id.
const SKILL_MENTION_RE_SOURCE = /\[@([^\]\n]+)\]\(#skill:([a-z0-9-]+)\)/

/** Every `[@label](#skill:name)` token in `input` (no allowlist filter — callers gate). */
export function parseSkillMentions(input: string): ParsedSkillMention[] {
  const out: ParsedSkillMention[] = []
  // Fresh /g regex per call so concurrent callers don't share `lastIndex`.
  const re = new RegExp(SKILL_MENTION_RE_SOURCE.source, "g")
  for (const m of input.matchAll(re)) {
    const start = m.index ?? 0
    const end = start + m[0].length
    out.push({ start, end, raw: m[0], label: m[1] ?? "", name: m[2] ?? "" })
  }
  return out
}

/** The literal text spliced for a chosen skill: `[@<label>](#skill:<name>)`. */
export function formatSkillInsertion(name: string, label: string): string {
  return `[@${label}](${SKILL_HREF_PREFIX}${name})`
}

/** Extract the skill id from a `#skill:<name>` href (used by the history renderer).
 *  Tolerates an encoded colon (`#skill%3A…`) some sanitizers emit. */
export function skillNameFromHref(href: string | undefined): string | null {
  if (!href) return null
  const m = /^#skill(?::|%3a)([a-z0-9-]+)$/i.exec(href)
  return m ? m[1] : null
}

/**
 * Strip an optional leading `skill:` so an explicit `@skill:exc` query filters
 * the skill section the same way a bare `@exc` does.
 */
export function skillQueryFromToken(token: string): string {
  const t = token.trim().toLowerCase()
  return t.startsWith("skill:") ? t.slice("skill:".length) : t
}

/** Whether the catalog entry `name` matches the `@`-section query (id + keywords).
 *  A keyword matches when it *contains* the query (so `doc` → `docx`, `exc` →
 *  `excel`). The reverse direction (query contains keyword) is intentionally NOT
 *  used — it over-matches, e.g. `webhook` would hit the `web` keyword. */
export function skillMatchesQuery(name: string, query: string): boolean {
  if (!query) return true
  const meta = SKILL_MENTION_CATALOG[name]
  if (name.toLowerCase().includes(query)) return true
  return !!meta?.keywords.some((k) => k.includes(query))
}
