/**
 * Shared session-status derivations and action contracts used by both the
 * title-bar status popover and the workspace session card, so the two never
 * drift on model resolution, cache stats, the `compact_context_now` result
 * shape, or the `/context` slash-command tagging. Each consumer keeps its own
 * toast / feedback UI — only the data + transport contracts live here.
 */
import { getTransport } from "@/lib/transport-provider"
import type { TFunction } from "i18next"
import type { ActiveModel, AvailableModel, Message } from "@/types/chat"
import type { CommandResult } from "./slash-commands/types"

/** Resolve the full {@link AvailableModel} for a session's active (provider, model). */
export function resolveCurrentModel(
  activeModel: ActiveModel | null | undefined,
  availableModels: AvailableModel[] | null | undefined,
): AvailableModel | null {
  if (!activeModel) return null
  return (
    (availableModels ?? []).find(
      (x) => x.providerId === activeModel.providerId && x.modelId === activeModel.modelId,
    ) ?? null
  )
}

export interface CacheStats {
  created: number
  read: number
  lastInput?: number
}

/** Cache stats from the latest assistant turn that carries usage (Anthropic). */
export function computeCacheStats(messages: Message[]): CacheStats | null {
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i]
    if (m.role !== "assistant" || !m.usage) continue
    const u = m.usage
    if (u.cacheCreationInputTokens == null && u.cacheReadInputTokens == null) return null
    return {
      created: u.cacheCreationInputTokens || 0,
      read: u.cacheReadInputTokens || 0,
      lastInput: u.lastInputTokens,
    }
  }
  return null
}

export interface CompactResult {
  tierApplied: number
  tokensBefore: number
  tokensAfter: number
  messagesAffected: number
  description?: string
}

export const COMPACT_CONTEXT_UPDATED_EVENT = "hope:compact-context-updated"

export interface CompactContextUpdatedDetail {
  sessionId: string
  result: CompactResult
}

/** Run the manual context compaction for a session. */
export async function compactContextNow(sessionId: string): Promise<CompactResult> {
  const result = await getTransport().call<CompactResult>("compact_context_now", { sessionId })
  if (typeof window !== "undefined") {
    window.dispatchEvent(
      new CustomEvent<CompactContextUpdatedDetail>(COMPACT_CONTEXT_UPDATED_EVENT, {
        detail: { sessionId, result },
      }),
    )
  }
  return result
}

/** Toast message describing a compaction result (saved tokens / affected turns). */
export function compactResultMessage(t: TFunction, result: CompactResult): string {
  if (result.messagesAffected > 0) {
    return String(
      t("chat.compactDone", {
        saved: result.tokensBefore - result.tokensAfter,
        affected: result.messagesAffected,
      }),
    )
  }

  switch (result.description) {
    case "no_messages":
      return String(t("chat.compactNoMessages", "There are no messages to compress"))
    case "summarization_not_applied":
      return String(t("chat.compactSummaryNotApplied", "Summary was needed, but was not completed"))
    case "summarization_not_applied_sync_compaction_only":
      return String(
        t("chat.compactSummaryNotAppliedSync", "Cleaned up context, but summary was not completed"),
      )
    case "cancelled":
      return String(t("chat.compactCancelled", "Compression was cancelled"))
    default:
      return String(t("chat.compactNoChange"))
  }
}

/**
 * Run the `/context` slash command and tag the result so the chat renders it as
 * a `/context` command output (the `_slashCommandText` round-trip contract).
 */
export async function runViewContext(
  sessionId: string,
  agentId?: string,
): Promise<CommandResult> {
  const result = await getTransport().call<CommandResult>("execute_slash_command", {
    sessionId,
    agentId,
    commandText: "/context",
  })
  result._slashCommandText = "/context"
  return result
}
