// Link resolution for markdown rendered inside the Help window.
//
// The manual's link shapes are a closed set (verified over the whole corpus):
// same-chapter `#anchor`, cross-chapter `NN-….md[#anchor]`, `README.md`
// (back to the index), the two language-switch links in the READMEs, links
// escaping the manual into the repo (`../deployment/…`, `../../README.md`),
// and external http(s). Anything unrecognized resolves to `none` — the
// renderer must NOT navigate on those.

import { HOPE_AGENT_URLS } from "@/lib/appMeta"

export type ManualLinkTarget =
  | { kind: "anchor"; anchor: string }
  | { kind: "chapter"; chapter: number; anchor?: string }
  | { kind: "language-switch" }
  | { kind: "external"; url: string }
  | { kind: "none" }

const CHAPTER_LINK = /^(?:\.\/)?(\d{2})-[^/#]*\.md$/

export function resolveManualLink(href: string, lang: "zh" | "en"): ManualLinkTarget {
  if (!href) return { kind: "none" }

  if (href.startsWith("#")) {
    const anchor = href.slice(1)
    return anchor ? { kind: "anchor", anchor } : { kind: "none" }
  }

  if (/^https?:\/\//i.test(href) || href.startsWith("mailto:")) {
    return { kind: "external", url: href }
  }

  const [path, anchor] = splitAnchor(href)

  // Language switch links live only in the two READMEs.
  if ((lang === "zh" && path === "en/README.md") || (lang === "en" && path === "../README.md")) {
    return { kind: "language-switch" }
  }

  if (path === "README.md" || path === "./README.md") {
    return anchor ? { kind: "chapter", chapter: 0, anchor } : { kind: "chapter", chapter: 0 }
  }

  const chapterMatch = CHAPTER_LINK.exec(path)
  if (chapterMatch) {
    const chapter = Number(chapterMatch[1])
    return anchor ? { kind: "chapter", chapter, anchor } : { kind: "chapter", chapter }
  }

  // Relative links that escape the manual (../architecture/, ../../README.md,
  // en/… siblings) → the corresponding file on GitHub.
  const repoPath = resolveRepoPath(path, lang)
  if (repoPath) {
    return { kind: "external", url: `${HOPE_AGENT_URLS.github}/tree/main/${repoPath}` }
  }

  return { kind: "none" }
}

function splitAnchor(href: string): [string, string | undefined] {
  const i = href.indexOf("#")
  if (i < 0) return [href, undefined]
  return [href.slice(0, i), href.slice(i + 1) || undefined]
}

/**
 * Rewrite every markdown link target in a chapter body into a form that
 * BOTH survives Streamdown's default rehype chain AND carries the resolved
 * navigation. rehype-harden (in the default chain, with no `defaultOrigin`)
 * replaces any bare RELATIVE href (`02-模型与Provider.md`) with a dead
 * "Blocked URL" span before our click handler ever sees it — fragments and
 * absolute http(s) URLs are the only shapes that pass. So chapter links
 * become `#ch:N[:anchor]`, the README language switchers become
 * `#lang-switch`, repo-escaping relative links become their absolute GitHub
 * URL, and unrecognized targets are left for harden to neutralize (they must
 * not navigate anyway).
 */
export function rewriteManualBody(body: string, lang: "zh" | "en"): string {
  return body.replace(/\]\(([^()\s]+)\)/g, (whole, target: string) => {
    const resolved = resolveManualLink(target, lang)
    switch (resolved.kind) {
      case "chapter":
        return `](#ch:${resolved.chapter}${resolved.anchor ? `:${resolved.anchor}` : ""})`
      case "language-switch":
        return "](#lang-switch)"
      case "external":
        return `](${resolved.url})`
      case "anchor":
      case "none":
        return whole
    }
  })
}

/** Resolve a rendered (post-`rewriteManualBody`) href back into a target. */
export function resolveRenderedHref(href: string): ManualLinkTarget {
  if (!href) return { kind: "none" }
  const chapterMatch = /^#ch:(\d+)(?::(.+))?$/.exec(href)
  if (chapterMatch) {
    const chapter = Number(chapterMatch[1])
    return chapterMatch[2]
      ? { kind: "chapter", chapter, anchor: chapterMatch[2] }
      : { kind: "chapter", chapter }
  }
  if (href === "#lang-switch") return { kind: "language-switch" }
  if (href.startsWith("#")) {
    const anchor = href.slice(1)
    return anchor ? { kind: "anchor", anchor } : { kind: "none" }
  }
  if (/^https?:\/\//i.test(href) || href.startsWith("mailto:")) {
    return { kind: "external", url: href }
  }
  return { kind: "none" }
}

/**
 * Resolve a manual-relative path against the manual's repo location into a
 * repo-relative path, or null when it escapes the repo root / is absolute.
 */
function resolveRepoPath(path: string, lang: "zh" | "en"): string | null {
  if (path.startsWith("/") || path.includes("\\")) return null
  const base = lang === "zh" ? ["docs", "user-guide"] : ["docs", "user-guide", "en"]
  const segments = [...base]
  for (const seg of path.split("/")) {
    if (seg === "" || seg === ".") continue
    if (seg === "..") {
      if (segments.length === 0) return null // escapes the repo root
      segments.pop()
    } else {
      segments.push(seg)
    }
  }
  if (segments.length === 0) return null
  return segments.join("/")
}
