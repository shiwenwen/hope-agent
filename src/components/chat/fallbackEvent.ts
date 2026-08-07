import type { FallbackEvent } from "@/types/chat"

function pickString(event: Record<string, unknown>, key: string): string | undefined {
  const value = event[key]
  return typeof value === "string" ? value : undefined
}

function pickNumber(event: Record<string, unknown>, key: string): number | undefined {
  const value = event[key]
  if (typeof value === "number" && Number.isFinite(value)) return value
  if (typeof value === "string") {
    const parsed = Number(value)
    if (Number.isFinite(parsed)) return parsed
  }
  return undefined
}

/** Normalize both live and persisted model-fallback payloads for one renderer. */
export function fallbackEventFromPayload(event: Record<string, unknown>): FallbackEvent {
  const model =
    pickString(event, "model") ??
    pickString(event, "model_id") ??
    pickString(event, "to_model") ??
    ""
  return {
    type: pickString(event, "type"),
    model,
    from_model: pickString(event, "from_model"),
    reason: pickString(event, "reason"),
    error: pickString(event, "error"),
    attempt: pickNumber(event, "attempt"),
    total: pickNumber(event, "total"),
    provider_id: pickString(event, "provider_id"),
    model_id: pickString(event, "model_id"),
  }
}
