import type { Message } from "@/types/chat"

// Generate a runtime-unique id for the `_clientId` slot on streaming
// placeholders. `crypto.randomUUID` is only exposed in *secure contexts*
// (HTTPS / localhost / file://) per the Web Crypto spec — Hope Agent's
// server mode runs plain HTTP, so a browser tab opened against a LAN IP
// (e.g. `http://192.168.x.x:8080`) sees `crypto.randomUUID === undefined`
// and `handleSend` would throw `TypeError` mid-flight, leaving the user
// message orphaned and the loading spinner stuck. The fallback below is
// fine because `_clientId` only needs uniqueness within the session — it's
// a React row key, never persisted, never sent over the wire.
export function generateClientId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID()
  }
  return `cid-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 11)}`
}

function messageStableId(msg: Message, index: number): string {
  // `_clientId` (set on streaming placeholders + transferred to the fresh
  // DB-loaded message at stream_end) takes priority over dbId so the React
  // row key stays stable across the placeholder→finalized transition.
  // Without this, `ts:assistant:<client_ts>` would flip to `db:<N>` the
  // moment fresh DB data lands, React would unmount/remount the row, and
  // the markdown subtree would rebuild — visible as a one-frame flicker.
  if (msg._clientId) return `cid:${msg._clientId}`
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

// Find a message row by its `data-message-key` value. Escapes only `"` and `\`
// — the minimum required for a well-formed quoted attribute selector — instead
// of relying on `CSS.escape`, which is missing in some WebViews and not
// guaranteed across jsdom versions.
export function findMessageRowByKey(
  scope: ParentNode,
  rowKey: string,
): HTMLElement | null {
  const escaped = rowKey.replace(/["\\]/g, "\\$&")
  return scope.querySelector<HTMLElement>(`[data-message-key="${escaped}"]`)
}
