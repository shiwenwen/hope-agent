/**
 * HelpMarkdown — one manual chapter rendered through the app's Streamdown
 * stack, with three help-specific behaviors layered on top WITHOUT touching
 * the shared MarkdownRenderer defaults:
 *
 * 1. Link pre-rewrite: `rewriteManualBody` turns every relative manual link
 *    into a fragment (`#ch:N[:anchor]`, `#lang-switch`) or absolute URL
 *    BEFORE rendering — rehype-harden in Streamdown's default chain would
 *    otherwise replace bare relative hrefs with dead "Blocked URL" spans.
 * 2. Heading `id`s: a rehype plugin injects the AUTHORITATIVE slugs shipped
 *    in the bundle (`headings[].slug`, computed once in Rust), matched by
 *    heading text; `manualSlug` is only the fallback for text the bundle
 *    doesn't know (formatted headings). Known limitation: duplicate heading
 *    texts resolve to the first occurrence's slug — the manual's `N.M`
 *    numbering keeps texts unique per chapter (Rust corpus test guards the
 *    anchors that exist).
 * 3. Link interception: a capture-phase click handler resolves the rendered
 *    hrefs and routes navigation to the Help view — the shared `MarkdownLink`
 *    component underneath never sees the click, so chat rendering behavior
 *    is untouched.
 */

import { useCallback, useMemo } from "react"
import { defaultRehypePlugins } from "streamdown"
import { code } from "@streamdown/code"
import { cjk } from "@streamdown/cjk"

import { MarkdownStreamdown } from "@/components/common/MarkdownRenderer"
import { manualSlug } from "@/lib/manual/manualSlug"
import {
  resolveRenderedHref,
  rewriteManualBody,
  type ManualLinkTarget,
} from "@/lib/manual/helpLinks"
import type { ManualHeading } from "@/lib/manual/manualTypes"

interface HastNode {
  type?: string
  tagName?: string
  value?: string
  properties?: Record<string, unknown>
  children?: HastNode[]
}

const HEADING_TAGS = new Set(["h1", "h2", "h3", "h4", "h5", "h6"])

function textOf(node: HastNode): string {
  if (node.type === "text") return node.value ?? ""
  return (node.children ?? []).map(textOf).join("")
}

function collectHeadings(node: HastNode, out: HastNode[]): void {
  if (node.tagName && HEADING_TAGS.has(node.tagName)) out.push(node)
  for (const child of node.children ?? []) collectHeadings(child, out)
}

/** Inject heading ids from the bundle's authoritative slug map. */
function makeHeadingIdsPlugin(slugByText: Map<string, string>) {
  return function helpHeadingIdsPlugin() {
    return (tree: HastNode) => {
      const headings: HastNode[] = []
      collectHeadings(tree, headings)
      for (const node of headings) {
        const text = textOf(node).trim()
        const slug = slugByText.get(text) ?? manualSlug(text)
        if (slug) node.properties = { ...node.properties, id: slug }
      }
    }
  }
}

const helpPlugins = { code, cjk }

interface HelpMarkdownProps {
  body: string
  lang: "zh" | "en"
  headings: ManualHeading[]
  onNavigate: (target: ManualLinkTarget) => void
}

export default function HelpMarkdown({ body, lang, headings, onNavigate }: HelpMarkdownProps) {
  const rendered = useMemo(() => rewriteManualBody(body, lang), [body, lang])

  // First occurrence wins on duplicate texts (see file header).
  const slugByText = useMemo(() => {
    const map = new Map<string, string>()
    for (const h of headings) {
      if (!map.has(h.text)) map.set(h.text, h.slug)
    }
    return map
  }, [headings])

  // Stable reference per chapter — Streamdown's per-block memo requires it
  // (see the matching comment in MarkdownRenderer).
  const rehypePlugins = useMemo(
    () => [...Object.values(defaultRehypePlugins), makeHeadingIdsPlugin(slugByText)],
    [slugByText],
  )

  const onClickCapture = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      const anchor = (e.target as HTMLElement).closest?.("a")
      if (!anchor) return
      e.preventDefault()
      e.stopPropagation()
      onNavigate(resolveRenderedHref(anchor.getAttribute("href") ?? ""))
    },
    [onNavigate],
  )

  return (
    <div className="markdown-content" onClickCapture={onClickCapture}>
      <MarkdownStreamdown plugins={helpPlugins} rehypePlugins={rehypePlugins}>
        {rendered}
      </MarkdownStreamdown>
    </div>
  )
}
