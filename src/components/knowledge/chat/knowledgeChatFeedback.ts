import { sanitizeDiagnosticText } from "@/lib/diagnosticRedaction"

export type KnowledgeChatLoadOperation =
  | "loadAgents"
  | "loadModels"
  | "loadAgentConfig"
  | "loadThread"
  | "loadDefaultThread"
  | "loadThreads"
  | "loadMoreThreads"
  | "loadMoreMessages"

export interface KnowledgeChatLoadIssue {
  operation: KnowledgeChatLoadOperation
  detail: string | null
}

export type KnowledgeChatFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

const KNOWLEDGE_CHAT_DIAGNOSTIC_MAX_CHARS = 420

export function knowledgeChatErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0
    ? sanitizeDiagnosticText(detail, KNOWLEDGE_CHAT_DIAGNOSTIC_MAX_CHARS)
    : null
}

export function knowledgeChatLoadIssue(
  operation: KnowledgeChatLoadOperation,
  error: unknown,
): KnowledgeChatLoadIssue {
  return { operation, detail: knowledgeChatErrorDetail(error) }
}

export function knowledgeChatIssueTitle(
  issue: KnowledgeChatLoadIssue,
  t: KnowledgeChatFeedbackTranslateFn,
): string {
  return t(knowledgeChatOperationKey(issue.operation), {
    defaultValue: knowledgeChatOperationFallback(issue.operation),
  })
}

export function knowledgeChatIssueDescription(
  issue: KnowledgeChatLoadIssue,
  t: KnowledgeChatFeedbackTranslateFn,
): string | null {
  if (!issue.detail) return null
  return t("knowledge.chatPanel.errors.detail", {
    defaultValue: "Details: {{error}}",
    error: issue.detail,
  })
}

function knowledgeChatOperationKey(operation: KnowledgeChatLoadOperation): string {
  switch (operation) {
    case "loadAgents":
      return "knowledge.chatPanel.errors.loadAgents"
    case "loadModels":
      return "knowledge.chatPanel.errors.loadModels"
    case "loadAgentConfig":
      return "knowledge.chatPanel.errors.loadAgentConfig"
    case "loadThread":
      return "knowledge.chatPanel.errors.loadThread"
    case "loadDefaultThread":
      return "knowledge.chatPanel.errors.loadDefaultThread"
    case "loadThreads":
      return "knowledge.chatPanel.errors.loadThreads"
    case "loadMoreThreads":
      return "knowledge.chatPanel.errors.loadMoreThreads"
    case "loadMoreMessages":
      return "knowledge.chatPanel.errors.loadMoreMessages"
  }
}

function knowledgeChatOperationFallback(operation: KnowledgeChatLoadOperation): string {
  switch (operation) {
    case "loadAgents":
      return "Couldn't load chat agents"
    case "loadModels":
      return "Couldn't load chat models"
    case "loadAgentConfig":
      return "Couldn't load this agent's chat settings"
    case "loadThread":
      return "Couldn't load this conversation"
    case "loadDefaultThread":
      return "Couldn't load the note conversation"
    case "loadThreads":
      return "Couldn't load conversation history"
    case "loadMoreThreads":
      return "Couldn't load more conversations"
    case "loadMoreMessages":
      return "Couldn't load earlier messages"
  }
}
