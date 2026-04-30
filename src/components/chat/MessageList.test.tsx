// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"

import type { Message } from "@/types/chat"
import MessageList from "./MessageList"

const virtualFeedMock = vi.hoisted(() => ({
  resumeAutoFollow: vi.fn(),
  pauseAutoFollow: vi.fn(),
  latestOptions: undefined as { forceFollowKey?: string | number | null } | undefined,
  state: {
    isAutoFollowPaused: false,
    hasUnseenOutput: false,
  },
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
        scrollToIndex: vi.fn(),
      },
      virtualItems: options.rows.map((row: { key: string }, index: number) => ({
        index,
        key: row.key,
        start: index * 100,
      })),
      totalSize: options.rows.length * 100,
      isAutoFollowPaused: virtualFeedMock.state.isAutoFollowPaused,
      hasUnseenOutput: virtualFeedMock.state.hasUnseenOutput,
      resumeAutoFollow: virtualFeedMock.resumeAutoFollow,
      pauseAutoFollow: virtualFeedMock.pauseAutoFollow,
    }
  }),
}))

vi.mock("./MessageBubble", () => ({
  default: ({ msg }: { msg: Message }) => <div data-testid="message-bubble">{msg.content}</div>,
}))

beforeEach(() => {
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    callback(performance.now())
    return 1
  })
  vi.stubGlobal("cancelAnimationFrame", vi.fn())
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
  vi.unstubAllGlobals()
  virtualFeedMock.latestOptions = undefined
  virtualFeedMock.state.isAutoFollowPaused = false
  virtualFeedMock.state.hasUnseenOutput = false
})

function baseMessage(patch: Partial<Message>): Message {
  return {
    role: "assistant",
    content: "",
    timestamp: "2026-04-26T00:00:00.000Z",
    ...patch,
  } as Message
}

describe("MessageList virtualization surface", () => {
  test("renders virtualized non-meta messages and load-more row", () => {
    const onLoadMore = vi.fn()
    render(
      <MessageList
        messages={[
          baseMessage({ role: "assistant", content: "hidden meta", isMeta: true }),
          baseMessage({ role: "user", content: "visible user message", dbId: 1 }),
        ]}
        loading={false}
        agents={[]}
        hasMore
        loadingMore={false}
        onLoadMore={onLoadMore}
        sessionId="s1"
      />,
    )

    expect(screen.getByText("visible user message")).toBeTruthy()
    expect(screen.queryByText("hidden meta")).toBeNull()

    fireEvent.click(screen.getByRole("button", { name: "chat.loadMore" }))
    expect(onLoadMore).toHaveBeenCalledTimes(1)
  })

  test("uses the incognito empty state for empty private sessions", () => {
    render(
      <MessageList
        messages={[]}
        loading={false}
        agents={[]}
        hasMore={false}
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s1"
        incognito
      />,
    )

    expect(screen.getByText("chat.incognitoEmptyTitle")).toBeTruthy()
    expect(screen.queryByText("chat.howCanIHelp")).toBeNull()
  })

  test("shows an icon-only scroll-to-bottom action when auto-follow is paused with unseen output", () => {
    virtualFeedMock.state.isAutoFollowPaused = true
    virtualFeedMock.state.hasUnseenOutput = true

    render(
      <MessageList
        messages={[baseMessage({ role: "assistant", content: "streaming answer", dbId: 1 })]}
        loading={false}
        agents={[]}
        hasMore={false}
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s1"
      />,
    )

    const button = screen.getByRole("button", { name: "chat.scrollToBottom" })
    expect(button.className).toContain("h-9")
    expect(button.className).toContain("w-9")
    expect(button.className).toContain("rounded-full")
    expect(button.className).toContain("cursor-pointer")
    expect(screen.queryByText("chat.jumpToLatest")).toBeNull()

    fireEvent.click(button)
    expect(virtualFeedMock.resumeAutoFollow).toHaveBeenCalledWith("smooth")
  })

  test("pauses auto-follow when jumping to a search result", () => {
    const onScrollTargetHandled = vi.fn()

    render(
      <MessageList
        messages={[baseMessage({ role: "assistant", content: "search hit", dbId: 42 })]}
        loading
        agents={[]}
        hasMore={false}
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s1"
        pendingScrollTarget={42}
        onScrollTargetHandled={onScrollTargetHandled}
      />,
    )

    expect(virtualFeedMock.pauseAutoFollow).toHaveBeenCalledWith(false)
    expect(onScrollTargetHandled).toHaveBeenCalledTimes(1)
  })

  test("forces auto-follow when the latest message is a newly sent user message", () => {
    render(
      <MessageList
        messages={[
          baseMessage({ role: "assistant", content: "previous answer", dbId: 1 }),
          baseMessage({ role: "user", content: "new question", dbId: 2 }),
        ]}
        loading={false}
        agents={[]}
        hasMore={false}
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s1"
      />,
    )

    expect(virtualFeedMock.latestOptions?.forceFollowKey).toBe("user-turn:1:db:2")
  })

  test("forces auto-follow when a newly sent user message already has an assistant placeholder", () => {
    render(
      <MessageList
        messages={[
          baseMessage({ role: "assistant", content: "previous answer", dbId: 1 }),
          baseMessage({
            role: "user",
            content: "new question",
            timestamp: "2026-04-26T00:01:00.000Z",
          }),
          baseMessage({
            role: "assistant",
            content: "",
            timestamp: "2026-04-26T00:01:00.001Z",
          }),
        ]}
        loading
        agents={[]}
        hasMore={false}
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s1"
      />,
    )

    expect(virtualFeedMock.latestOptions?.forceFollowKey).toBe(
      "user-turn:1:ts:2026-04-26T00:01:00.000Z",
    )
  })
})
