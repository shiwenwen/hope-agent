import { sanitizeMemoryDiagnosticText } from "./memoryDiagnosticRedaction"

export type MemoryExperienceOperation =
  | "search"
  | "loadMore"
  | "saveEpisode"
  | "saveProcedure"
  | "promoteEpisode"
  | "openSourceEpisode"
  | "openExperience"
  | "focusExperience"
  | "loadHistory"
  | "archiveExperience"
  | "restoreExperience"

export type MemoryExperienceFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface MemoryExperienceOperationErrorToast {
  title: string
  description?: string
}

const MEMORY_EXPERIENCE_DIAGNOSTIC_MAX_CHARS = 420

export function memoryExperienceOperationErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? memoryExperienceDiagnosticText(detail) : null
}

export function memoryExperienceDiagnosticText(
  value: string,
  maxChars = MEMORY_EXPERIENCE_DIAGNOSTIC_MAX_CHARS,
): string {
  return sanitizeMemoryDiagnosticText(value, maxChars)
}

export function memoryExperienceOperationErrorToast(
  operation: MemoryExperienceOperation,
  t: MemoryExperienceFeedbackTranslateFn,
  error: unknown,
): MemoryExperienceOperationErrorToast {
  const detail = memoryExperienceOperationErrorDetail(error)
  const title = t(`settings.memoryExperienceErrors.${operation}`, {
    defaultValue: memoryExperienceOperationFallback(operation),
  })
  if (!detail) return { title }
  return {
    title,
    description: t("settings.memoryExperienceErrors.detail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

function memoryExperienceOperationFallback(operation: MemoryExperienceOperation): string {
  switch (operation) {
    case "search":
      return "Failed to search experience memory"
    case "loadMore":
      return "Failed to load more experience memory"
    case "saveEpisode":
      return "Failed to save episode"
    case "saveProcedure":
      return "Failed to save workflow"
    case "promoteEpisode":
      return "Failed to promote episode"
    case "openSourceEpisode":
      return "Failed to open source episode"
    case "openExperience":
      return "Failed to open experience memory"
    case "focusExperience":
      return "Failed to focus experience memory"
    case "loadHistory":
      return "Failed to load experience history"
    case "archiveExperience":
      return "Failed to archive experience"
    case "restoreExperience":
      return "Failed to restore experience"
  }
}
