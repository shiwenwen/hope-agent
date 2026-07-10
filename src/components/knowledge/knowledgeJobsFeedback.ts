import { sanitizeDiagnosticText } from "@/lib/diagnosticRedaction"

export type KnowledgeJobsOperation = "loadJobs" | "cancelJob" | "retryJob" | "clearJob"

export type KnowledgeJobsFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface KnowledgeJobsErrorToast {
  title: string
  description?: string
}

export function knowledgeJobsErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? sanitizeDiagnosticText(detail) : null
}

export function knowledgeJobsErrorToast(
  operation: KnowledgeJobsOperation,
  t: KnowledgeJobsFeedbackTranslateFn,
  error: unknown,
): KnowledgeJobsErrorToast {
  const title = t(knowledgeJobsOperationKey(operation), {
    defaultValue: knowledgeJobsOperationFallback(operation),
  })
  const detail = knowledgeJobsErrorDetail(error)
  if (!detail) return { title }
  return {
    title,
    description: t("knowledge.jobs.errorDetail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

export function knowledgeJobsActionOperation(
  action: "cancel" | "retry" | "clear",
): KnowledgeJobsOperation {
  switch (action) {
    case "cancel":
      return "cancelJob"
    case "retry":
      return "retryJob"
    case "clear":
      return "clearJob"
  }
}

function knowledgeJobsOperationKey(operation: KnowledgeJobsOperation): string {
  switch (operation) {
    case "loadJobs":
      return "knowledge.jobs.loadFailed"
    case "cancelJob":
      return "knowledge.jobs.cancelFailed"
    case "retryJob":
      return "knowledge.jobs.retryFailed"
    case "clearJob":
      return "knowledge.jobs.clearFailed"
  }
}

function knowledgeJobsOperationFallback(operation: KnowledgeJobsOperation): string {
  switch (operation) {
    case "loadJobs":
      return "Couldn't load rebuild tasks"
    case "cancelJob":
      return "Couldn't cancel rebuild task"
    case "retryJob":
      return "Couldn't retry rebuild task"
    case "clearJob":
      return "Couldn't clear rebuild task"
  }
}
