import { embeddingModelDiagnosticText } from "@/components/settings/embedding-models/embeddingModelFeedback"

export type KnowledgeEmbeddingBadgeTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export function knowledgeEmbeddingLoadErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? embeddingModelDiagnosticText(detail) : null
}

export function knowledgeEmbeddingUnavailableTip(
  t: KnowledgeEmbeddingBadgeTranslateFn,
  detail: string | null,
): string {
  const title = t("knowledge.embeddingStatusUnavailableTip", {
    defaultValue: "Vector search status is unavailable — open settings",
  })
  if (!detail) return title
  return `${title} · ${t("knowledge.embeddingStatusErrorDetail", {
    defaultValue: "Details: {{error}}",
    error: detail,
  })}`
}
