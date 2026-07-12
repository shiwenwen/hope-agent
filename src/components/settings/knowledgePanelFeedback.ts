import { embeddingModelDiagnosticText } from "./embedding-models/embeddingModelFeedback"

export type KnowledgePanelOperation =
  | "loadEmbedding"
  | "activateEmbedding"
  | "disableEmbedding"
  | "rebuildEmbedding"
  | "cancelReembed"
  | "retryReembed"

export type KnowledgePanelFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface KnowledgePanelOperationErrorToast {
  title: string
  description?: string
}

export type KnowledgeCompileAgentOperation =
  | "loadConfig"
  | "loadAgents"
  | "saveAgent"

export type KnowledgePassiveRecallOperation =
  | "load"
  | "save"
  | "toggle"

export type KnowledgeSearchRankingOperation =
  | "load"
  | "save"
  | "restore"

export type KnowledgeChunkOperation =
  | "load"
  | "save"

export type KnowledgeMediaRetentionOperation =
  | "load"
  | "save"
  | "toggle"

export function knowledgePanelErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? embeddingModelDiagnosticText(detail) : null
}

export function knowledgePanelOperationErrorToast(
  operation: KnowledgePanelOperation,
  t: KnowledgePanelFeedbackTranslateFn,
  error: unknown,
): KnowledgePanelOperationErrorToast {
  const detail = knowledgePanelErrorDetail(error)
  const title = t(`settings.knowledgeEmbedding.errors.${operation}`, {
    defaultValue: knowledgePanelOperationFallback(operation),
  })
  if (!detail) return { title }
  return {
    title,
    description: t("settings.knowledgeEmbedding.errors.detail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

export function knowledgePanelOperationErrorText(
  operation: KnowledgePanelOperation,
  t: KnowledgePanelFeedbackTranslateFn,
  error: unknown,
): string {
  const toast = knowledgePanelOperationErrorToast(operation, t, error)
  return [toast.title, toast.description].filter(Boolean).join("\n")
}

export function knowledgeCompileAgentOperationErrorToast(
  operation: KnowledgeCompileAgentOperation,
  t: KnowledgePanelFeedbackTranslateFn,
  error: unknown,
): KnowledgePanelOperationErrorToast {
  const detail = knowledgePanelErrorDetail(error)
  const title = t(`settings.knowledgeCompile.errors.${operation}`, {
    defaultValue: knowledgeCompileAgentOperationFallback(operation),
  })
  if (!detail) return { title }
  return {
    title,
    description: t("settings.knowledgeCompile.errors.detail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

export function knowledgePassiveRecallOperationErrorToast(
  operation: KnowledgePassiveRecallOperation,
  t: KnowledgePanelFeedbackTranslateFn,
  error: unknown,
): KnowledgePanelOperationErrorToast {
  const detail = knowledgePanelErrorDetail(error)
  const title = t(`settings.knowledgePassiveRecall.errors.${operation}`, {
    defaultValue: knowledgePassiveRecallOperationFallback(operation),
  })
  if (!detail) return { title }
  return {
    title,
    description: t("settings.knowledgePassiveRecall.errors.detail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

export function knowledgeSearchRankingOperationErrorToast(
  operation: KnowledgeSearchRankingOperation,
  t: KnowledgePanelFeedbackTranslateFn,
  error: unknown,
): KnowledgePanelOperationErrorToast {
  const detail = knowledgePanelErrorDetail(error)
  const title = t(`settings.knowledgeSearch.errors.${operation}`, {
    defaultValue: knowledgeSearchRankingOperationFallback(operation),
  })
  if (!detail) return { title }
  return {
    title,
    description: t("settings.knowledgeSearch.errors.detail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

export function knowledgeChunkOperationErrorToast(
  operation: KnowledgeChunkOperation,
  t: KnowledgePanelFeedbackTranslateFn,
  error: unknown,
): KnowledgePanelOperationErrorToast {
  const detail = knowledgePanelErrorDetail(error)
  const title = t(`settings.knowledgeChunk.errors.${operation}`, {
    defaultValue: knowledgeChunkOperationFallback(operation),
  })
  if (!detail) return { title }
  return {
    title,
    description: t("settings.knowledgeChunk.errors.detail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

export function knowledgeMediaRetentionOperationErrorToast(
  operation: KnowledgeMediaRetentionOperation,
  t: KnowledgePanelFeedbackTranslateFn,
  error: unknown,
): KnowledgePanelOperationErrorToast {
  const detail = knowledgePanelErrorDetail(error)
  const title = t(`settings.knowledgeMediaRetention.errors.${operation}`, {
    defaultValue: knowledgeMediaRetentionOperationFallback(operation),
  })
  if (!detail) return { title }
  return {
    title,
    description: t("settings.knowledgeMediaRetention.errors.detail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

function knowledgePanelOperationFallback(operation: KnowledgePanelOperation): string {
  switch (operation) {
    case "loadEmbedding":
      return "Failed to load knowledge vector search settings"
    case "activateEmbedding":
      return "Failed to activate knowledge vector search"
    case "disableEmbedding":
      return "Failed to disable knowledge vector search"
    case "rebuildEmbedding":
      return "Failed to start knowledge vector rebuild"
    case "cancelReembed":
      return "Failed to cancel knowledge vector rebuild"
    case "retryReembed":
      return "Failed to retry knowledge vector rebuild"
  }
}

function knowledgeCompileAgentOperationFallback(operation: KnowledgeCompileAgentOperation): string {
  switch (operation) {
    case "loadConfig":
      return "Failed to load source-to-note agent setting"
    case "loadAgents":
      return "Failed to load source-to-note agent list"
    case "saveAgent":
      return "Failed to save source-to-note agent setting"
  }
}

function knowledgePassiveRecallOperationFallback(operation: KnowledgePassiveRecallOperation): string {
  switch (operation) {
    case "load":
      return "Failed to load passive related notes setting"
    case "save":
      return "Failed to save passive related notes setting"
    case "toggle":
      return "Failed to update passive related notes toggle"
  }
}

function knowledgeSearchRankingOperationFallback(operation: KnowledgeSearchRankingOperation): string {
  switch (operation) {
    case "load":
      return "Failed to load knowledge search ranking settings"
    case "save":
      return "Failed to save knowledge search ranking settings"
    case "restore":
      return "Failed to restore knowledge search ranking defaults"
  }
}

function knowledgeChunkOperationFallback(operation: KnowledgeChunkOperation): string {
  switch (operation) {
    case "load":
      return "Failed to load knowledge chunking settings"
    case "save":
      return "Failed to save knowledge chunking settings"
  }
}

function knowledgeMediaRetentionOperationFallback(
  operation: KnowledgeMediaRetentionOperation,
): string {
  switch (operation) {
    case "load":
      return "Failed to load original media retention settings"
    case "save":
      return "Failed to save original media retention settings"
    case "toggle":
      return "Failed to update original media retention toggle"
  }
}
