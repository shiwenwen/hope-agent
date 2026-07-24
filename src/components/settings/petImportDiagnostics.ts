import type { PetImportSource } from "@/types/pet"

export interface PetImportFailureDiagnostic {
  command: "pet_import_preview_cmd"
  sourceKind: PetImportSource["kind"]
  errorKind:
    | "invalid_args"
    | "timeout"
    | "permission_denied"
    | "network_error"
    | "invoke_failed"
    | `pet_${string}`
  field?: string
}

const SAFE_FIELD = /^[A-Za-z][A-Za-z0-9_]{0,63}$/
const SAFE_PET_ERROR = /\bpet_[a-z0-9_]{1,80}\b/i

function errorText(error: unknown): string {
  if (error instanceof Error) return error.message
  return String(error)
}

/**
 * Convert an invoke rejection into bounded, non-sensitive diagnostics. Raw
 * errors may contain a local path, URL, upload id, or provider response, so
 * they must never be handed directly to the persistent frontend logger.
 */
export function petImportFailureDiagnostic(
  source: PetImportSource,
  error: unknown,
): PetImportFailureDiagnostic {
  const text = errorText(error)
  const lower = text.toLowerCase()
  const base = {
    command: "pet_import_preview_cmd" as const,
    sourceKind: source.kind,
  }
  if (lower.includes("invalid args")) {
    const candidate = text.match(/missing field\s+[`'"]([A-Za-z][A-Za-z0-9_]{0,63})[`'"]/i)?.[1]
    return {
      ...base,
      errorKind: "invalid_args",
      ...(candidate && SAFE_FIELD.test(candidate) ? { field: candidate } : {}),
    }
  }
  const petCode = text.match(SAFE_PET_ERROR)?.[0]?.toLowerCase()
  if (petCode) return { ...base, errorKind: petCode as `pet_${string}` }
  if (lower.includes("timeout") || lower.includes("timed out")) {
    return { ...base, errorKind: "timeout" }
  }
  if (lower.includes("permission denied") || lower.includes("not permitted")) {
    return { ...base, errorKind: "permission_denied" }
  }
  if (lower.includes("network") || lower.includes("fetch") || lower.includes("connection")) {
    return { ...base, errorKind: "network_error" }
  }
  return { ...base, errorKind: "invoke_failed" }
}
