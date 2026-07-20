// Wire types for the built-in user manual (Help Center) — mirror of the
// serde(camelCase) structs in crates/ha-core/src/manual/mod.rs.

export interface ManualHeading {
  /** 1–6 */
  level: number
  text: string
  /** GitHub-style anchor slug — authoritative source is the Rust side. */
  slug: string
  /** 1-based source line. */
  line: number
}

export interface ManualChapter {
  /** 1..=13 for chapters, 0 for the README index page. */
  number: number
  title: string
  /** Full markdown body, verbatim. */
  body: string
  headings: ManualHeading[]
}

export interface ManualBundle {
  lang: string
  /** "zh" | "en" — the manual language actually served. */
  effectiveLang: string
  chapters: ManualChapter[]
}

export interface ManualSearchHit {
  chapter: number
  chapterTitle: string
  anchor: string | null
  line: number
  /** Matched line with STX/ETX (/) hit markers. */
  snippet: string
  score: number
}
