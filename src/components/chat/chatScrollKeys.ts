import type { Message } from "@/types/chat"

function messageStableId(msg: Message, index: number): string {
  if (typeof msg.dbId === "number") return `db:${msg.dbId}`
  // useChatStream creates an optimistic user message and an assistant
  // placeholder back-to-back before either lands in the DB; their `new
  // Date().toISOString()` stamps frequently collide on the same millisecond,
  // so role must be part of the fallback key to keep React row keys distinct.
  if (msg.timestamp) return `ts:${msg.role}:${msg.timestamp}`
  return `idx:${index}`
}

export function getMessageRowKey(msg: Message, index: number): string {
  return `message:${messageStableId(msg, index)}`
}

export function getLatestUserTurnKey(messages: Message[]): string | null {
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const msg = messages[i]
    if (msg.role !== "user") continue
    return `user-turn:${messageStableId(msg, i)}`
  }
  return null
}
