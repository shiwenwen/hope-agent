import { sanitizeDiagnosticText } from "@/lib/diagnosticRedaction"

export type KnowledgeQuickRewriteFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface KnowledgeQuickRewriteErrorToast {
  title: string
  description?: string
}

export type KnowledgeQuickRewriteOperation = "generate" | "loadModels"

export function knowledgeQuickRewriteErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? sanitizeDiagnosticText(detail) : null
}

export function knowledgeQuickRewriteOperationError(
  operation: KnowledgeQuickRewriteOperation,
  t: KnowledgeQuickRewriteFeedbackTranslateFn,
  error: unknown,
): KnowledgeQuickRewriteErrorToast {
  const title = t(knowledgeQuickRewriteOperationKey(operation), {
    defaultValue: knowledgeQuickRewriteOperationFallback(operation),
  })
  const detail = knowledgeQuickRewriteErrorDetail(error)
  if (!detail) return { title }
  return {
    title,
    description: t("knowledge.quickRewrite.errorDetail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

export function knowledgeQuickRewriteErrorToast(
  t: KnowledgeQuickRewriteFeedbackTranslateFn,
  error: unknown,
): KnowledgeQuickRewriteErrorToast {
  return knowledgeQuickRewriteOperationError("generate", t, error)
}

function knowledgeQuickRewriteOperationKey(operation: KnowledgeQuickRewriteOperation): string {
  switch (operation) {
    case "generate":
      return "knowledge.quickRewrite.failed"
    case "loadModels":
      return "knowledge.quickRewrite.modelLoadFailed"
  }
}

function knowledgeQuickRewriteOperationFallback(operation: KnowledgeQuickRewriteOperation): string {
  switch (operation) {
    case "generate":
      return "Rewrite failed"
    case "loadModels":
      return "Couldn't load rewrite models"
  }
}
