// @vitest-environment jsdom

import { afterEach, describe, expect, test, vi } from "vitest"
import { cleanup, render } from "@testing-library/react"

import type { Message } from "@/types/chat"
import QuickChatMessages from "./QuickChatMessages"

const virtualFeedMock = vi.hoisted(() => ({
  latestOptions: undefined as { forceFollowKey?: string | number | null } | undefined,
}))

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) => key,
  }),
}))

vi.mock("@/components/common/useVirtualFeed", () => ({
  useVirtualFeed: vi.fn((options: { rows: { key: string }[]; forceFollowKey?: string | number | null }) => {
    virtualFeedMock.latestOptions = options
    return {
      scrollRef: { current: null },
      virtualizer: {
        measureElement: vi.fn(),
      },
      virtualItems: options.rows.map((row, index) => ({
        index,
        key: row.key,
        start: index * 80,
      })),
      totalSize: options.rows.length * 80,
      isAutoFollowPaused: false,
      hasUnseenOutput: false,
      resumeAutoFollow: vi.fn(),
    }
  }),
}))

vi.mock("@/components/common/MarkdownRenderer", () => ({
  default: ({ content }: { content: string }) => <div>{content}</div>,
}))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
  virtualFeedMock.latestOptions = undefined
})

function baseMessage(patch: Partial<Message>): Message {
  return {
    role: "assistant",
    content: "",
    timestamp: "2026-04-26T00:00:00.000Z",
    ...patch,
  } as Message
}

describe("QuickChatMessages auto-follow", () => {
  test("forces auto-follow when the latest message is a newly sent user message", () => {
    render(
      <QuickChatMessages
        messages={[baseMessage({ role: "user", content: "hello", dbId: 1 })]}
        loading={false}
        sessionId="s1"
      />,
    )

    expect(virtualFeedMock.latestOptions?.forceFollowKey).toBe("user-turn:0:db:1")
  })

  test("forces auto-follow when a newly sent user message already has an assistant placeholder", () => {
    render(
      <QuickChatMessages
        messages={[
          baseMessage({
            role: "user",
            content: "hello",
            timestamp: "2026-04-26T00:01:00.000Z",
          }),
          baseMessage({
            role: "assistant",
            content: "",
            timestamp: "2026-04-26T00:01:00.001Z",
          }),
        ]}
        loading
        sessionId="s1"
      />,
    )

    expect(virtualFeedMock.latestOptions?.forceFollowKey).toBe(
      "user-turn:0:ts:2026-04-26T00:01:00.000Z",
    )
  })
})
