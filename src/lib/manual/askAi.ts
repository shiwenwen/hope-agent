// "Ask AI" bridge: Help window → main chat composer.
//
// The manual excerpt is delivered as a staged message-quote chip (the
// existing PendingMessageQuote mechanism), not by overwriting the composer
// text. Two hops:
//
//   Help window ── emitAskAi ──► App (listenAskAi: switch to chat view)
//                                  └─► deliverAskAi ─► ChatScreen
//                                      (subscribeAskAiQuotes: stage chip)
//
// Desktop uses Tauri's app-global event bus (reaches every window); the Web
// GUI's help tab uses a BroadcastChannel to the main tab. The App→ChatScreen
// hop buffers through a module-level queue so a quote arriving while another
// view is active is staged as soon as the chat screen mounts.

import { isTauriMode } from "@/lib/transport"
import { logger } from "@/lib/logger"

const EVENT = "help:ask-ai"
const CHANNEL = "hope-agent-help-ask-ai"

export interface AskAiPayload {
  /** Quote chip content (manual excerpt / chapter reference). */
  text: string
}

/** How (whether) the excerpt reached the main surface. */
export type AskAiDelivery = "desktop" | "delivered" | "no-listener"

/** How long the web path waits for a main-tab ack before reporting loss. */
const ACK_TIMEOUT_MS = 400

/** Help-window side: send the excerpt and pull the main surface forward. */
export async function emitAskAi(payload: AskAiPayload): Promise<AskAiDelivery> {
  if (!isTauriMode()) {
    // BroadcastChannel is fire-and-forget; without an ack a help tab with no
    // main-app tab open would silently drop the excerpt. The App-side
    // listener acks every ask, so no ack within the timeout = nobody home.
    return new Promise<AskAiDelivery>((resolve) => {
      try {
        const ch = new BroadcastChannel(CHANNEL)
        const done = (result: AskAiDelivery) => {
          ch.close()
          resolve(result)
        }
        const timer = setTimeout(() => done("no-listener"), ACK_TIMEOUT_MS)
        ch.onmessage = (e) => {
          if ((e.data as { type?: string } | undefined)?.type === "ack") {
            clearTimeout(timer)
            done("delivered")
          }
        }
        ch.postMessage({ type: "ask", text: payload.text })
      } catch (e) {
        logger.error("help", "askAi::emit", "Ask-AI broadcast failed", { error: e })
        resolve("no-listener")
      }
    })
  }
  try {
    const { emit } = await import("@tauri-apps/api/event")
    await emit(EVENT, payload)
    const { WebviewWindow } = await import("@tauri-apps/api/webviewWindow")
    const main = await WebviewWindow.getByLabel("main")
    if (main) {
      await main.show()
      await main.unminimize()
      await main.setFocus()
    }
  } catch (e) {
    logger.error("help", "askAi::emit", "Ask-AI emit failed", { error: e })
  }
  return "desktop"
}

/** App side: external event → callback (used to switch to the chat view). */
export function listenAskAi(cb: (payload: AskAiPayload) => void): () => void {
  if (!isTauriMode()) {
    const ch = new BroadcastChannel(CHANNEL)
    ch.onmessage = (e) => {
      const data = e.data as { type?: string; text?: unknown } | undefined
      if (data?.type === "ack") return
      const text = data?.text
      if (typeof text === "string" && text.trim()) {
        // Ack first so the help tab can report delivery even if cb throws.
        ch.postMessage({ type: "ack" })
        cb({ text })
      }
    }
    return () => ch.close()
  }
  let unlisten: (() => void) | null = null
  let cancelled = false
  void import("@tauri-apps/api/event").then(({ listen }) =>
    listen<AskAiPayload>(EVENT, (event) => {
      const text = event.payload?.text
      if (typeof text === "string" && text.trim()) cb({ text })
    }).then((fn) => {
      if (cancelled) fn()
      else unlisten = fn
    }),
  )
  return () => {
    cancelled = true
    unlisten?.()
  }
}

// ── App → ChatScreen hand-off (mount-race-free) ─────────────────────────

let pendingQuotes: string[] = []
let subscriber: ((text: string) => void) | null = null

/** App side: queue a quote for the chat screen (drained on mount). */
export function deliverAskAi(text: string): void {
  if (subscriber) subscriber(text)
  else pendingQuotes.push(text)
}

/** ChatScreen side: receive queued + future quotes while mounted. */
export function subscribeAskAiQuotes(cb: (text: string) => void): () => void {
  subscriber = cb
  const drained = pendingQuotes
  pendingQuotes = []
  drained.forEach(cb)
  return () => {
    if (subscriber === cb) subscriber = null
  }
}
