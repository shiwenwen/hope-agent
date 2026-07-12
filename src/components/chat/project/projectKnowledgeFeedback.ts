import { sanitizeDiagnosticText } from "@/lib/diagnosticRedaction"

const PROJECT_KNOWLEDGE_DIAGNOSTIC_MAX_CHARS = 420

export type ProjectKnowledgeFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface ProjectKnowledgeToast {
  title: string
  description?: string
}

export function projectKnowledgeErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  if (!detail) return null
  return sanitizeDiagnosticText(detail, PROJECT_KNOWLEDGE_DIAGNOSTIC_MAX_CHARS)
}

export function projectKnowledgeLoadErrorToast(
  t: ProjectKnowledgeFeedbackTranslateFn,
  error: unknown,
): ProjectKnowledgeToast {
  return projectKnowledgeErrorToast(t, error, {
    titleKey: "project.knowledge.loadFailed",
    titleFallback: "Failed to load project knowledge spaces",
  })
}

export function projectKnowledgeUpdateErrorToast(
  t: ProjectKnowledgeFeedbackTranslateFn,
  error: unknown,
): ProjectKnowledgeToast {
  return projectKnowledgeErrorToast(t, error, {
    titleKey: "project.knowledge.updateFailed",
    titleFallback: "Failed to update project knowledge space",
  })
}

function projectKnowledgeErrorToast(
  t: ProjectKnowledgeFeedbackTranslateFn,
  error: unknown,
  title: { titleKey: string; titleFallback: string },
): ProjectKnowledgeToast {
  const detail = projectKnowledgeErrorDetail(error)
  const toast = {
    title: t(title.titleKey, { defaultValue: title.titleFallback }),
  }
  if (!detail) return toast
  return {
    ...toast,
    description: t("project.knowledge.errorDetail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}
