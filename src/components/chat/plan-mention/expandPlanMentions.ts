// At send-time, resolve only provenance-bearing `@plan:` selections into a
// concrete `file_path` attachment. Pasted/manually typed lookalikes remain
// ordinary text. The backend independently re-resolves the typed target and
// freezes the exact bytes before the first provider attempt.

import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { ChatAttachment } from "@/lib/transport"
import type { PlanMentionResolution } from "@/components/plans/types"
import type { ComposerMentionBinding } from "@/components/chat/mentions/typedMentions"
import { parsePlanMentions } from "./parsePlanMentions"

export interface PlanMentionAttachment extends ChatAttachment {
  file_path: string
}

export async function expandPlanMentionsToAttachments(
  input: string,
  bindings: ComposerMentionBinding[],
): Promise<PlanMentionAttachment[]> {
  const tokens = bindings.flatMap((binding) => {
    if (binding.kind !== "plan") return []
    if (input.slice(binding.start, binding.end) !== binding.raw) return []
    const parsed = parsePlanMentions(binding.raw)
    if (parsed.length !== 1 || parsed[0].raw.length !== binding.raw.length) return []
    if (`${parsed[0].shortId}:v${parsed[0].version}` !== binding.targetId.toLowerCase()) return []
    return parsed
  })
  if (tokens.length === 0) return []

  // Resolve all tokens concurrently — each call is an independent RPC.
  // Failures stay scoped to their token; success-only results feed the
  // dedup-by-file_path pass below in original token order.
  const results = await Promise.all(
    tokens.map(async (token) => {
      try {
        const resolved = await getTransport().call<PlanMentionResolution>("resolve_plan_mention", {
          shortId: token.shortId,
          version: token.version,
        })
        return { token, resolved }
      } catch {
        logger.warn("ui", "expandPlanMentions", "Failed to resolve one typed plan reference")
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
      source: "plan_mention",
      file_path: resolved.filePath,
    })
  }

  if (out.length > 0) {
    logger.info("ui", "expandPlanMentions", `attaching ${out.length} typed plan reference(s)`)
  }
  return out
}
