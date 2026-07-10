import { DEFAULT_AGENT_ID } from "@/types/tools"
import { sanitizeMemoryDiagnosticText } from "./memoryDiagnosticRedaction"

const MEMORY_USE_IN_REPLIES_DIAGNOSTIC_MAX_CHARS = 420

export type MemoryUseInRepliesOperation = "load" | "update"

export type MemoryUseInRepliesTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export function normalizeDefaultMemoryAgentId(id: unknown): string {
  return typeof id === "string" && id.trim().length > 0 ? id.trim() : DEFAULT_AGENT_ID
}

export function memoryUseInRepliesErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  if (!detail) return null
  return memoryUseInRepliesDiagnosticText(detail)
}

export function memoryUseInRepliesDiagnosticText(
  value: string,
  maxChars = MEMORY_USE_IN_REPLIES_DIAGNOSTIC_MAX_CHARS,
): string {
  return sanitizeMemoryDiagnosticText(value, maxChars)
}

export function formatMemoryUseInRepliesError(
  t: MemoryUseInRepliesTranslateFn,
  operation: MemoryUseInRepliesOperation,
  error: unknown,
): string {
  const detail = memoryUseInRepliesErrorDetail(error)
  if (operation === "load") {
    if (!detail) {
      return t("settings.memoryUseInRepliesLoadFailed", {
        defaultValue: "Could not check active recall",
      })
    }
    return t("settings.memoryUseInRepliesLoadError", {
      defaultValue: "Could not check active recall: {{error}}",
      error: detail,
    })
  }
  if (!detail) {
    return t("settings.memoryUseInRepliesUpdateFailed", {
      defaultValue: "Could not update active recall",
    })
  }
  return t("settings.memoryUseInRepliesError", {
    defaultValue: "Could not update active recall: {{error}}",
    error: detail,
  })
}

export function memoryUseInRepliesErrorDescription(
  t: MemoryUseInRepliesTranslateFn,
  error: unknown,
): string | undefined {
  const detail = memoryUseInRepliesErrorDetail(error)
  if (!detail) return undefined
  return t("settings.memoryUseInRepliesErrorDetail", {
    defaultValue: "Details: {{error}}",
    error: detail,
  })
}
