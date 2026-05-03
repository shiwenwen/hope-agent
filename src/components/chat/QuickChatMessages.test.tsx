// @vitest-environment jsdom

import React, { createContext } from "react"
import type { ReactNode } from "react"
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react"

import type { Message } from "@/types/chat"
import QuickChatMessages from "./QuickChatMessages"

const virtuosoMock = vi.hoisted(() => ({
  scrollToIndex: vi.fn(),
  scrollTo: vi.fn(),
  scrollBy: vi.fn(),
  scrollIntoView: vi.fn(),
  autoscrollToBottom: vi.fn(),
  getState: vi.fn(),
  latestProps: undefined as
    | {
        startReached?: (index: number) => void
        atBottomStateChange?: (atBottom: boolean) => void
        firstItemIndex?: number
      }
    | undefined,
}))

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

vi.mock("react-virtuoso", () => {
  const Virtuoso = React.forwardRef(function VirtuosoMock(
    props: {
      data?: unknown[]
      firstItemIndex?: number
      components?: {
        Header?: React.ComponentType
        Footer?: React.ComponentType
      }
      itemContent?: (index: number, item: unknown) => React.ReactNode
      computeItemKey?: (index: number, item: unknown) => string | number
      startReached?: (index: number) => void
      atBottomStateChange?: (atBottom: boolean) => void
    },
    ref,
  ) {
    React.useLayoutEffect(() => {
      virtuosoMock.latestProps = {
        startReached: props.startReached,
        atBottomStateChange: props.atBottomStateChange,
        firstItemIndex: props.firstItemIndex,
      }
    })

    React.useImperativeHandle(ref, () => ({
      scrollToIndex: virtuosoMock.scrollToIndex,
      scrollTo: virtuosoMock.scrollTo,
      scrollBy: virtuosoMock.scrollBy,
      scrollIntoView: virtuosoMock.scrollIntoView,
      autoscrollToBottom: virtuosoMock.autoscrollToBottom,
      getState: virtuosoMock.getState,
    }))

    const data = props.data ?? []
    const Header = props.components?.Header
    const Footer = props.components?.Footer

    return (
      <div data-testid="virtuoso-mock">
        {Header ? <Header /> : null}
        {data.map((item, i) => (
          <div key={props.computeItemKey?.(i, item) ?? i} data-testid="virtuoso-item">
            {props.itemContent?.(i, item)}
          </div>
        ))}
        {Footer ? <Footer /> : null}
      </div>
    )
  })
  return { Virtuoso, VirtuosoMockContext: createContext(undefined) }
})

vi.mock("@/components/common/MarkdownRenderer", () => ({
  default: ({ content }: { content: string }) => <div>{content}</div>,
}))

vi.mock("@/components/ui/tooltip", () => ({
  IconTip: ({ children }: { children: ReactNode }) => children,
}))

beforeEach(() => {
  virtuosoMock.scrollToIndex.mockClear()
  virtuosoMock.scrollTo.mockClear()
  virtuosoMock.scrollBy.mockClear()
  virtuosoMock.scrollIntoView.mockClear()
  virtuosoMock.autoscrollToBottom.mockClear()
  virtuosoMock.getState.mockClear()
  virtuosoMock.latestProps = undefined
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

function baseMessage(patch: Partial<Message>): Message {
  return {
    role: "assistant",
    content: "",
    timestamp: "2026-04-26T00:00:00.000Z",
    ...patch,
  } as Message
}

describe("QuickChatMessages", () => {
  test("returns null when there are no messages", () => {
    const { container } = render(
      <QuickChatMessages messages={[]} loading={false} sessionId="s1" />,
    )
    expect(container.firstChild).toBeNull()
  })

  test("renders user and assistant messages with their content", () => {
    render(
      <QuickChatMessages
        messages={[
          baseMessage({ role: "user", content: "ping", dbId: 1 }),
          baseMessage({ role: "assistant", content: "pong", dbId: 2 }),
        ]}
        loading={false}
        sessionId="s1"
      />,
    )

    expect(screen.getByText("ping")).toBeTruthy()
    expect(screen.getByText("pong")).toBeTruthy()
  })

  test("renders the LoadMoreRow header and triggers onLoadMore on click", () => {
    const onLoadMore = vi.fn()
    render(
      <QuickChatMessages
        messages={[baseMessage({ role: "user", content: "hi", dbId: 1 })]}
        loading={false}
        sessionId="s1"
        hasMore
        loadingMore={false}
        onLoadMore={onLoadMore}
      />,
    )

    fireEvent.click(screen.getByRole("button", { name: "chat.loadMore" }))
    expect(onLoadMore).toHaveBeenCalledTimes(1)
  })

  test("startReached calls onLoadMore when hasMore and not loadingMore", () => {
    const onLoadMore = vi.fn()
    render(
      <QuickChatMessages
        messages={[baseMessage({ role: "user", content: "hi", dbId: 1 })]}
        loading={false}
        sessionId="s1"
        hasMore
        loadingMore={false}
        onLoadMore={onLoadMore}
      />,
    )

    virtuosoMock.latestProps?.startReached?.(0)
    expect(onLoadMore).toHaveBeenCalledTimes(1)
  })

  test('renders the "view full chat" link when sessionId and onNavigateToSession are provided', () => {
    const onNavigateToSession = vi.fn()
    render(
      <QuickChatMessages
        messages={[baseMessage({ role: "user", content: "hi", dbId: 1 })]}
        loading={false}
        sessionId="s1"
        onNavigateToSession={onNavigateToSession}
      />,
    )

    fireEvent.click(screen.getByText("quickChat.viewFullChat"))
    expect(onNavigateToSession).toHaveBeenCalledWith("s1")
  })

  test("shows the jump-to-bottom button when not at bottom and triggers scrollToIndex on click", () => {
    render(
      <QuickChatMessages
        messages={[baseMessage({ role: "assistant", content: "streaming", dbId: 1 })]}
        loading
        sessionId="s1"
      />,
    )

    act(() => {
      virtuosoMock.latestProps?.atBottomStateChange?.(false)
    })

    const button = screen.getByRole("button", { name: "chat.scrollToBottom" })
    expect(button.className).toContain("h-8")
    expect(button.className).toContain("w-8")
    expect(button.className).toContain("rounded-full")

    fireEvent.click(button)
    const calls = virtuosoMock.scrollToIndex.mock.calls
    expect(calls.length).toBeGreaterThan(0)
    expect(calls[calls.length - 1]?.[0]).toMatchObject({ index: "LAST", align: "end" })
  })

  test("forces a scroll when a new user message arrives", () => {
    const { rerender } = render(
      <QuickChatMessages
        messages={[baseMessage({ role: "assistant", content: "old", dbId: 1 })]}
        loading={false}
        sessionId="s1"
      />,
    )

    virtuosoMock.scrollToIndex.mockClear()

    rerender(
      <QuickChatMessages
        messages={[
          baseMessage({ role: "assistant", content: "old", dbId: 1 }),
          baseMessage({ role: "user", content: "new question", dbId: 2 }),
        ]}
        loading={false}
        sessionId="s1"
      />,
    )

    expect(virtuosoMock.scrollToIndex).toHaveBeenCalled()
    const calls = virtuosoMock.scrollToIndex.mock.calls
    expect(calls[calls.length - 1]?.[0]).toMatchObject({
      align: "start",
      behavior: "smooth",
    })
  })
})
