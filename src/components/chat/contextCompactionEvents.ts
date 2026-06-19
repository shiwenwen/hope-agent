function objectField(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null
  return value as Record<string, unknown>
}

export function parseEventPayload(content: string): Record<string, unknown> | null {
  try {
    return objectField(JSON.parse(content))
  } catch {
    return null
  }
}

export function contextCompactionData(payload: Record<string, unknown>): Record<string, unknown> {
  // Fall back to an empty object (not the envelope) when `data` is absent:
  // classification fields (description / tier_applied / phase) always live
  // under `data`, never at the top level, so reading the envelope would
  // misclassify a data-less notice.
  return objectField(payload.data) ?? {}
}

export function isContextCompactionPayload(
  payload: Record<string, unknown> | null,
): payload is Record<string, unknown> {
  return (
    payload?.type === "context_compacted" ||
    payload?.type === "context_compaction_progress"
  )
}

export function isContextCompactionStartPayload(payload: Record<string, unknown>): boolean {
  if (payload.type === "context_compaction_progress") return true
  if (payload.type !== "context_compacted") return false
  const data = contextCompactionData(payload)
  return data.description === "summarizing" || data.description === "emergency_compacting"
}

export function contextCompactionNoticePriority(payload: Record<string, unknown>): number {
  const data = contextCompactionData(payload)
  const description = typeof data.description === "string" ? data.description : ""
  const phase = typeof data.phase === "string" ? data.phase : ""
  const kind = typeof data.kind === "string" ? data.kind : ""
  const tier = typeof data.tier_applied === "number" ? data.tier_applied : 0

  if (phase === "failed") return 5
  if (
    kind === "emergency" ||
    description === "emergency_compact" ||
    description === "emergency_compacting" ||
    tier >= 4
  ) {
    return 4
  }
  if (description === "summarized" || description === "summarization_needed" || tier === 3) {
    return 3
  }
  if (description === "context_pruned" || tier === 2) return 2
  return 1
}

export function shouldReplaceContextCompactionNotice(
  previousPayload: Record<string, unknown> | null,
  nextPayload: Record<string, unknown>,
): boolean {
  if (!isContextCompactionPayload(previousPayload)) return true
  if (isContextCompactionStartPayload(previousPayload)) return true
  return (
    contextCompactionNoticePriority(nextPayload) >= contextCompactionNoticePriority(previousPayload)
  )
}
