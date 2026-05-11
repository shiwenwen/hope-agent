// At send-time, resolve every `@plan:` token in the user input into a
// concrete `file_path` attachment by calling the backend
// `resolve_plan_mention` RPC. Failed resolutions (deleted session,
// ambiguous short_id, missing version) are logged and skipped — the
// raw `@plan:` text stays in the user message so the LLM can still see
// the reference, just without the resolved content.

import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { ChatAttachment } from "@/lib/transport"
import type { PlanMentionResolution } from "@/components/plans/types"
import { parsePlanMentions } from "./parsePlanMentions"

export interface PlanMentionAttachment extends ChatAttachment {
  file_path: string
}

export async function expandPlanMentionsToAttachments(
  input: string,
): Promise<PlanMentionAttachment[]> {
  const tokens = parsePlanMentions(input)
  if (tokens.length === 0) return []

  // Resolve all tokens concurrently — each call is an independent RPC.
  // Failures stay scoped to their token; success-only results feed the
  // dedup-by-file_path pass below in original token order.
  const results = await Promise.all(
    tokens.map(async (token) => {
      try {
        const resolved = await getTransport().call<PlanMentionResolution>(
          "resolve_plan_mention",
          { shortId: token.shortId, version: token.version },
        )
        return { token, resolved }
      } catch (e) {
        logger.warn(
          "ui",
          "expandPlanMentions",
          `Failed to resolve ${token.raw}; leaving as text`,
          e,
        )
        return null
      }
    }),
  )

  const out: PlanMentionAttachment[] = []
  const seenPaths = new Set<string>()
  for (const r of results) {
    if (!r) continue
    const { resolved } = r
    if (!resolved.filePath || seenPaths.has(resolved.filePath)) continue
    seenPaths.add(resolved.filePath)
    const baseName = resolved.title
      ? `${resolved.title}.md`
      : (resolved.filePath.split("/").filter(Boolean).pop() ?? "plan.md")
    out.push({
      name: baseName,
      mime_type: "text/markdown",
      file_path: resolved.filePath,
    })
  }

  if (out.length > 0) {
    logger.info(
      "ui",
      "expandPlanMentions",
      `attaching ${out.length} plan mention file(s)`,
      { files: out.map((a) => a.name) },
    )
  }
  return out
}
