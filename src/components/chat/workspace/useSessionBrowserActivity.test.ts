import { describe, expect, it } from "vitest"
import type { Message } from "@/types/chat"
import {
  aggregateSessionBrowserActivity,
  messagesHaveBrowserActivity,
} from "./useSessionBrowserActivity"

function browserMsg(
  patch: Partial<ReturnType<typeof aggregateSessionBrowserActivity>[number]> = {},
): Message {
  return {
    role: "assistant",
    content: "",
    contentBlocks: [
      {
        type: "tool_call",
        tool: {
          callId: patch.callId ?? "call-1",
          name: "browser",
          arguments: "{}",
          metadata: {
            kind: "browser_activity",
            action: patch.action ?? "navigate",
            op: patch.op ?? "go",
            targetId: patch.targetId ?? "tab-1",
            url: patch.url ?? "https://example.com",
            title: patch.title ?? "Example",
            backend: patch.backend ?? "extension",
            sessionId: patch.sessionId ?? "s1",
            callId: patch.callId ?? "call-1",
            at: patch.at ?? 1000,
          },
        },
      },
    ],
  }
}

describe("aggregateSessionBrowserActivity", () => {
  it("extracts browser activity metadata from tool calls", () => {
    const result = aggregateSessionBrowserActivity([browserMsg()])
    expect(result).toEqual([
      {
        action: "navigate",
        op: "go",
        targetId: "tab-1",
        url: "https://example.com",
        title: "Example",
        backend: "extension",
        sessionId: "s1",
        callId: "call-1",
        at: 1000,
      },
    ])
  })

  it("falls back to the tool call id when metadata lacks callId", () => {
    const msg = browserMsg({ callId: null })
    const block = msg.contentBlocks?.[0]
    if (block?.type === "tool_call" && block.tool.metadata?.kind === "browser_activity") {
      block.tool.callId = "tool-call"
      block.tool.metadata.callId = null
    }
    expect(aggregateSessionBrowserActivity([msg])[0].callId).toBe("tool-call")
  })

  it("reports whether the loaded window has browser activity", () => {
    expect(messagesHaveBrowserActivity([browserMsg()])).toBe(true)
    expect(messagesHaveBrowserActivity([{ role: "assistant", content: "plain" }])).toBe(false)
  })
})
