import { Fragment, useMemo, type ReactNode } from "react"
import { SkillMentionText } from "@/components/chat/skill-mention/SkillMentionText"
import { findAutoLinkMatches } from "@/lib/autoLink"
import { MarkdownLink } from "./MarkdownRenderer"

interface PlainTextRendererProps {
  content: string
}

function renderTextWithLinks(content: string) {
  const matches = findAutoLinkMatches(content)
  if (matches.length === 0) return <SkillMentionText text={content} />

  const nodes: ReactNode[] = []
  let cursor = 0
  for (const match of matches) {
    if (match.start > cursor) {
      nodes.push(
        <Fragment key={`text-${cursor}`}>
          <SkillMentionText text={content.slice(cursor, match.start)} />
        </Fragment>,
      )
    }
    nodes.push(
      <MarkdownLink key={`link-${match.start}`} href={match.href}>
        {match.text}
      </MarkdownLink>,
    )
    cursor = match.end
  }
  if (cursor < content.length) {
    nodes.push(
      <Fragment key={`text-${cursor}`}>
        <SkillMentionText text={content.slice(cursor)} />
      </Fragment>,
    )
  }
  return nodes
}

export default function PlainTextRenderer({ content }: PlainTextRendererProps) {
  const rendered = useMemo(() => renderTextWithLinks(content), [content])
  if (!content) return null
  return <div className="markdown-content plain-text-content">{rendered}</div>
}
