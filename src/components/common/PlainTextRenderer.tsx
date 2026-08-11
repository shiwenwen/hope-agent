import { Fragment, useMemo, type ReactNode } from "react"
import { SkillMentionText } from "@/components/chat/skill-mention/SkillMentionText"
import { findAutoLinkMatches } from "@/lib/autoLink"
import { MarkdownLink } from "./MarkdownRenderer"
import type { ComposerMentionBinding } from "@/components/chat/mentions/typedMentions"

interface PlainTextRendererProps {
  content: string
  typedMentions?: ComposerMentionBinding[]
}

function renderTextWithLinks(content: string, typedMentions: ComposerMentionBinding[]) {
  const matches = findAutoLinkMatches(content)
  if (matches.length === 0) {
    return <SkillMentionText text={content} typedMentions={typedMentions} />
  }

  const nodes: ReactNode[] = []
  let cursor = 0
  for (const match of matches) {
    if (match.start > cursor) {
      nodes.push(
        <Fragment key={`text-${cursor}`}>
          <SkillMentionText
            text={content.slice(cursor, match.start)}
            typedMentions={typedMentions}
            sourceOffset={cursor}
          />
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
        <SkillMentionText
          text={content.slice(cursor)}
          typedMentions={typedMentions}
          sourceOffset={cursor}
        />
      </Fragment>,
    )
  }
  return nodes
}

export default function PlainTextRenderer({ content, typedMentions = [] }: PlainTextRendererProps) {
  const rendered = useMemo(
    () => renderTextWithLinks(content, typedMentions),
    [content, typedMentions],
  )
  if (!content) return null
  return <div className="markdown-content plain-text-content">{rendered}</div>
}
