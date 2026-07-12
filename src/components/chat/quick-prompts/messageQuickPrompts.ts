import type { Message } from "@/types/chat"

export function isQuickPromptEligibleUserMessage(msg: Message): boolean {
  return (
    msg.role === "user" &&
    !msg.fromAgentId &&
    !msg.isSubagentResult &&
    !msg.isCronTrigger &&
    !msg.isWakeupTrigger &&
    !msg.isProcessNotification &&
    !msg.isWorkflowResult &&
    !msg.isPlanTrigger &&
    !msg.planComment &&
    !msg.isMeta &&
    !msg.slashEvent &&
    !msg.channelInbound &&
    msg.content.trim().length > 0
  )
}

export function recentUserInputHistory(messages: Message[], limit = 50): string[] {
  const seen = new Set<string>()
  const out: string[] = []
  for (let i = messages.length - 1; i >= 0; i--) {
    const msg = messages[i]
    if (!msg || !isQuickPromptEligibleUserMessage(msg)) continue
    const content = msg.content.trim()
    if (seen.has(content)) continue
    seen.add(content)
    out.push(content)
    if (out.length >= limit) break
  }
  return out
}
