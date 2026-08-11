/**
 * Render typed mention tokens as inline chips for compact single-line surfaces
 * (the timeline sticky-anchor pill) that aren't full markdown. Every token
 * needs an exact typed provenance span; unknown or unbound lookalikes remain
 * literal source text.
 */

import { Fragment, type ReactNode } from "react"

import { AgentMentionChip } from "../agent-mention/AgentMentionChip"
import { parseAgentMentions } from "../agent-mention/agentTokens"
import { CapabilityMentionChip } from "../capability-mention/CapabilityMentionChip"
import { SkillMentionChip } from "./SkillMentionChip"
import { isSkillMentionName, parseSkillMentions } from "./skillTokens"
import { parseCapabilityMentions, type ComposerMentionBinding } from "../mentions/typedMentions"

export function SkillMentionText({
  text,
  typedMentions = [],
  sourceOffset = 0,
}: {
  text: string
  typedMentions?: ComposerMentionBinding[]
  /** Offset of `text` inside the original message when a parent renderer
   * splits around auto-linked URLs. */
  sourceOffset?: number
}) {
  const spans = [
    ...parseSkillMentions(text).map((span) => ({ ...span, kind: "skill" as const })),
    ...parseAgentMentions(text).map((span) => ({ ...span, kind: "agent" as const })),
    ...parseCapabilityMentions(text).map((span) => ({
      ...span,
      capabilityKind: span.kind,
      kind: "capability" as const,
    })),
  ]
    .filter((span) =>
      typedMentions.some(
        (mention) =>
          mention.kind === (span.kind === "capability" ? span.capabilityKind : span.kind) &&
          mention.start === sourceOffset + span.start &&
          mention.end === sourceOffset + span.end &&
          mention.raw === span.raw &&
          (span.kind !== "capability" || mention.targetId === span.targetId),
      ),
    )
    .sort((a, b) => a.start - b.start)
  if (spans.length === 0) return <>{text}</>

  const out: ReactNode[] = []
  let cursor = 0
  spans.forEach((span, i) => {
    if (span.start < cursor) return
    if (span.start > cursor) {
      out.push(<Fragment key={`t-${i}`}>{text.slice(cursor, span.start)}</Fragment>)
    }
    if (span.kind === "skill" && isSkillMentionName(span.name)) {
      out.push(<SkillMentionChip key={`s-${i}`} name={span.name} />)
    } else if (span.kind === "agent") {
      out.push(<AgentMentionChip key={`a-${i}`} agentId={span.agentId} fallbackName={span.label} />)
    } else if (span.kind === "capability") {
      out.push(
        <CapabilityMentionChip
          key={`c-${i}`}
          kind={span.capabilityKind}
          targetId={span.targetId}
          fallbackName={span.label}
        />,
      )
    } else {
      out.push(<Fragment key={`f-${i}`}>{`@${span.label}`}</Fragment>)
    }
    cursor = span.end
  })
  if (cursor < text.length) out.push(<Fragment key="tail">{text.slice(cursor)}</Fragment>)
  return <>{out}</>
}
