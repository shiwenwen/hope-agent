import type { Message } from "@/types/chat"

function userTurnIdentity(msg: Message, index: number): string {
  if (typeof msg.dbId === "number") return `db:${msg.dbId}`
  if (msg.timestamp) return `ts:${msg.timestamp}`
  return `idx:${index}`
}

function messageIdentity(msg: Message, index: number): string {
  if (typeof msg.dbId === "number") return `db:${msg.dbId}`
  if (msg.timestamp) return `ts:${msg.timestamp}`
  return `idx:${index}`
}

export function getLatestUserTurnKey(messages: Message[]): string | null {
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const msg = messages[i]
    if (msg.role !== "user") continue
    return `user-turn:${userTurnIdentity(msg, i)}`
  }
  return null
}

export function getLatestMessageOutputKey(messages: Message[]): string | null {
  const index = messages.length - 1
  const msg = messages[index]
  if (!msg) return null

  return [
    "latest",
    messageIdentity(msg, index),
    msg.role,
    msg.content.length,
    msg.contentBlocks?.length ?? 0,
    msg.toolCalls?.length ?? 0,
    msg.thinking?.length ?? 0,
  ].join(":")
}
