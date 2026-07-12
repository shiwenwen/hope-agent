import { sanitizeDiagnosticText } from "@/lib/diagnosticRedaction"

export type KnowledgeGraphOperation = "loadGraph" | "saveLayout" | "resetLayout"

export type KnowledgeGraphFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface KnowledgeGraphErrorToast {
  title: string
  description?: string
}

export function knowledgeGraphErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? sanitizeDiagnosticText(detail) : null
}

export function knowledgeGraphErrorToast(
  operation: KnowledgeGraphOperation,
  t: KnowledgeGraphFeedbackTranslateFn,
  error: unknown,
): KnowledgeGraphErrorToast {
  const title = t(knowledgeGraphOperationKey(operation), {
    defaultValue: knowledgeGraphOperationFallback(operation),
  })
  const detail = knowledgeGraphErrorDetail(error)
  if (!detail) return { title }
  return {
    title,
    description: t("knowledge.graph.errorDetail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

function knowledgeGraphOperationKey(operation: KnowledgeGraphOperation): string {
  switch (operation) {
    case "loadGraph":
      return "knowledge.graph.loadFailed"
    case "saveLayout":
      return "knowledge.graph.saveLayoutFailed"
    case "resetLayout":
      return "knowledge.graph.resetLayoutFailed"
  }
}

function knowledgeGraphOperationFallback(operation: KnowledgeGraphOperation): string {
  switch (operation) {
    case "loadGraph":
      return "Couldn't load knowledge graph"
    case "saveLayout":
      return "Couldn't save graph layout"
    case "resetLayout":
      return "Couldn't reset graph layout"
  }
}
