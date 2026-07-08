import { useMemo } from "react"
import type { BrowserActivityMetadata, Message } from "@/types/chat"
import { iterateMessageToolCalls } from "./useSessionFileChanges"

export type BrowserActivityAction = BrowserActivityMetadata["action"]

export interface SessionBrowserActivity {
  action: BrowserActivityAction
  op?: string | null
  targetId?: string | null
  url?: string | null
  title?: string | null
  backend?: string | null
  sessionId?: string | null
  callId?: string | null
  at?: number | null
}

function keyForActivity(activity: SessionBrowserActivity, fallbackIndex: number): string {
  return activity.callId
    ? `call:${activity.callId}`
    : [
        "browser",
        activity.at ?? fallbackIndex,
        activity.action,
        activity.op ?? "",
        activity.targetId ?? "",
        activity.url ?? "",
      ].join(":")
}

/**
 * Browser tool activity in the loaded message window. Full-history aggregation
 * is done by `session/artifacts.rs`; this live tail keeps the current streaming
 * turn visible before the tool row is persisted.
 */
export function aggregateSessionBrowserActivity(messages: Message[]): SessionBrowserActivity[] {
  const entries = new Map<string, SessionBrowserActivity>()
  let seq = 0

  for (const message of messages) {
    for (const tool of iterateMessageToolCalls(message)) {
      const meta = tool.metadata
      if (meta?.kind !== "browser_activity") continue
      const activity: SessionBrowserActivity = {
        action: meta.action,
        op: meta.op,
        targetId: meta.targetId,
        url: meta.url,
        title: meta.title,
        backend: meta.backend,
        sessionId: meta.sessionId,
        callId: meta.callId ?? tool.callId,
        at: meta.at,
      }
      entries.set(keyForActivity(activity, seq), activity)
      seq += 1
    }
  }

  return [...entries.values()]
}

export function messagesHaveBrowserActivity(messages: Message[]): boolean {
  for (const message of messages) {
    for (const tool of iterateMessageToolCalls(message)) {
      if (tool.metadata?.kind === "browser_activity") return true
    }
  }
  return false
}

export function useSessionBrowserActivity(messages: Message[]): SessionBrowserActivity[] {
  return useMemo(() => aggregateSessionBrowserActivity(messages), [messages])
}
