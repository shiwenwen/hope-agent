import { sanitizeDiagnosticText } from "@/lib/diagnosticRedaction"

export type KnowledgeSpriteOperation = "loadConfig" | "saveToggle" | "observe"

export type KnowledgeSpriteFeedbackTranslateFn = (
  key: string,
  options?: Record<string, unknown>,
) => string

export interface KnowledgeSpriteErrorToast {
  title: string
  description?: string
}

export function knowledgeSpriteErrorDetail(error: unknown): string | null {
  if (error == null) return null
  const value =
    error instanceof Error ? error.message : typeof error === "string" ? error : String(error)
  const detail = value.trim()
  return detail.length > 0 ? sanitizeDiagnosticText(detail) : null
}

export function knowledgeSpriteErrorToast(
  operation: KnowledgeSpriteOperation,
  t: KnowledgeSpriteFeedbackTranslateFn,
  error: unknown,
): KnowledgeSpriteErrorToast {
  const title = t(knowledgeSpriteOperationKey(operation), {
    defaultValue: knowledgeSpriteOperationFallback(operation),
  })
  const detail = knowledgeSpriteErrorDetail(error)
  if (!detail) return { title }
  return {
    title,
    description: t("knowledge.sprite.errorDetail", {
      defaultValue: "Details: {{error}}",
      error: detail,
    }),
  }
}

function knowledgeSpriteOperationKey(operation: KnowledgeSpriteOperation): string {
  switch (operation) {
    case "loadConfig":
      return "knowledge.sprite.loadFailed"
    case "saveToggle":
      return "knowledge.sprite.toggleFailed"
    case "observe":
      return "knowledge.sprite.observeFailed"
  }
}

function knowledgeSpriteOperationFallback(operation: KnowledgeSpriteOperation): string {
  switch (operation) {
    case "loadConfig":
      return "Couldn't load sprite mode"
    case "saveToggle":
      return "Couldn't update sprite mode"
    case "observe":
      return "Couldn't ask the sprite for a suggestion"
  }
}
