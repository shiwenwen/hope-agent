import { sanitizeDiagnosticText } from "@/lib/diagnosticRedaction"

export type NoteSourceReferenceOperation = "loadRefs" | "readSource"

export type NoteSourceReferenceTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface NoteSourceReferenceErrorMessage {
  title: string
  description?: string
}

export function noteSourceReferenceErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? sanitizeDiagnosticText(detail) : null
}

export function noteSourceReferenceErrorMessage(
  operation: NoteSourceReferenceOperation,
  t: NoteSourceReferenceTranslateFn,
  error: unknown,
): NoteSourceReferenceErrorMessage {
  const title =
    operation === "loadRefs"
      ? t("knowledge.sources.sourceRefsFailed", {
          defaultValue: "Couldn't load source references",
        })
      : t("knowledge.sources.readFailed", {
          defaultValue: "Couldn't open source",
        })
  const detail = noteSourceReferenceErrorDetail(error)
  if (!detail) return { title }
  return {
    title,
    description: t("knowledge.sources.sourceErrorDetail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}
