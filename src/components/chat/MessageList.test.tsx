// @vitest-environment jsdom

import React, { createContext } from "react"
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react"

import type { Message } from "@/types/chat"
import MessageList from "./MessageList"
import type { AskUserQuestionGroup } from "./ask-user/AskUserQuestionBlock"
import type { PlanCardData } from "./plan-mode/PlanCardBlock"

// Capture the latest props handed to <Virtuoso/> + a ref handle stub so we can
// assert against scrollToIndex calls without spinning up a real DOM scroller
// (jsdom has no ResizeObserver). We render Header/items/Footer eagerly so
// component composition is easy to test.
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
          <div
            key={props.computeItemKey?.(i, item) ?? i}
            data-testid="virtuoso-item"
          >
            {props.itemContent?.(i, item)}
          </div>
        ))}
        {Footer ? <Footer /> : null}
      </div>
    )
  })
  return {
    Virtuoso,
    VirtuosoMockContext: createContext(undefined),
  }
})

vi.mock("./MessageBubble", () => ({
  default: ({ msg }: { msg: Message }) => <div data-testid="message-bubble">{msg.content}</div>,
}))

vi.mock("./ask-user/AskUserQuestionBlock", () => ({
  default: ({ group }: { group: AskUserQuestionGroup }) => (
    <div data-testid="ask-user-block">{group.requestId}</div>
  ),
}))

vi.mock("./plan-mode/PlanCardBlock", () => ({
  default: ({ data }: { data: PlanCardData }) => (
    <div data-testid="plan-card-block">{data.title}</div>
  ),
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

describe("MessageList", () => {
  test("renders non-meta messages and hides isMeta entries", () => {
    render(
      <MessageList
        messages={[
          baseMessage({ role: "assistant", content: "hidden meta", isMeta: true }),
          baseMessage({ role: "user", content: "visible user message", dbId: 1 }),
        ]}
        loading={false}
        agents={[]}
        hasMore={false}
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s1"
      />,
    )

    expect(screen.getByText("visible user message")).toBeTruthy()
    expect(screen.queryByText("hidden meta")).toBeNull()
  })

  test("renders LoadMoreRow in the header when hasMore is true and triggers onLoadMore on click", () => {
    const onLoadMore = vi.fn()
    render(
      <MessageList
        messages={[baseMessage({ role: "user", content: "first message", dbId: 1 })]}
        loading={false}
        agents={[]}
        hasMore
        loadingMore={false}
        onLoadMore={onLoadMore}
        sessionId="s1"
      />,
    )

    fireEvent.click(screen.getByRole("button", { name: "chat.loadMore" }))
    expect(onLoadMore).toHaveBeenCalledTimes(1)
  })

  test("startReached calls onLoadMore when hasMore and not loadingMore", () => {
    const onLoadMore = vi.fn()
    render(
      <MessageList
        messages={[baseMessage({ role: "user", content: "msg", dbId: 1 })]}
        loading={false}
        agents={[]}
        hasMore
        loadingMore={false}
        onLoadMore={onLoadMore}
        sessionId="s1"
      />,
    )

    virtuosoMock.latestProps?.startReached?.(0)
    expect(onLoadMore).toHaveBeenCalledTimes(1)
  })

  test("startReached is a no-op while loadingMore is true", () => {
    const onLoadMore = vi.fn()
    render(
      <MessageList
        messages={[baseMessage({ role: "user", content: "msg", dbId: 1 })]}
        loading={false}
        agents={[]}
        hasMore
        loadingMore
        onLoadMore={onLoadMore}
        sessionId="s1"
      />,
    )

    virtuosoMock.latestProps?.startReached?.(0)
    expect(onLoadMore).not.toHaveBeenCalled()
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

  test("uses the default empty state for empty non-private sessions", () => {
    render(
      <MessageList
        messages={[]}
        loading={false}
        agents={[]}
        hasMore={false}
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s1"
      />,
    )

    expect(screen.getByText("chat.howCanIHelp")).toBeTruthy()
    expect(screen.queryByText("chat.incognitoEmptyTitle")).toBeNull()
  })

  test("renders ask-user, plan-card and plan-running blocks in the footer", () => {
    const askUserGroup: AskUserQuestionGroup = {
      requestId: "ask-1",
      questions: [],
    } as unknown as AskUserQuestionGroup
    const planCard: PlanCardData = { title: "test plan" }

    render(
      <MessageList
        messages={[baseMessage({ role: "user", content: "ping", dbId: 1 })]}
        loading={false}
        agents={[]}
        hasMore={false}
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s1"
        pendingQuestionGroup={askUserGroup}
        planCardData={planCard}
        planState="executing"
        planSubagentRunning
      />,
    )

    expect(screen.getByTestId("ask-user-block")).toBeTruthy()
    expect(screen.getByTestId("plan-card-block")).toBeTruthy()
    expect(screen.getByText("planMode.planningInProgress")).toBeTruthy()
  })

  test("does not render plan-card while plan state is off or planning", () => {
    const planCard: PlanCardData = { title: "test plan" }
    const { rerender } = render(
      <MessageList
        messages={[baseMessage({ role: "user", content: "ping", dbId: 1 })]}
        loading={false}
        agents={[]}
        hasMore={false}
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s1"
        planCardData={planCard}
        planState="off"
      />,
    )
    expect(screen.queryByTestId("plan-card-block")).toBeNull()

    rerender(
      <MessageList
        messages={[baseMessage({ role: "user", content: "ping", dbId: 1 })]}
        loading={false}
        agents={[]}
        hasMore={false}
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s1"
        planCardData={planCard}
        planState="planning"
      />,
    )
    expect(screen.queryByTestId("plan-card-block")).toBeNull()
  })

  test("scrolls to a search target by dbId and reports it as handled", () => {
    const onScrollTargetHandled = vi.fn()
    render(
      <MessageList
        messages={[
          baseMessage({ role: "assistant", content: "earlier", dbId: 41 }),
          baseMessage({ role: "assistant", content: "search hit", dbId: 42 }),
        ]}
        loading={false}
        agents={[]}
        hasMore={false}
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s1"
        pendingScrollTarget={42}
        onScrollTargetHandled={onScrollTargetHandled}
      />,
    )

    expect(virtuosoMock.scrollToIndex).toHaveBeenCalledTimes(1)
    // dbId 42 is at data index 1; scrollToIndex receives a data-relative index
    expect(virtuosoMock.scrollToIndex).toHaveBeenCalledWith({ index: 1, align: "center" })
    expect(onScrollTargetHandled).toHaveBeenCalledTimes(1)
  })

  test("shows the jump-to-bottom button while loading and not at bottom", () => {
    render(
      <MessageList
        messages={[baseMessage({ role: "assistant", content: "streaming", dbId: 1 })]}
        loading
        agents={[]}
        hasMore={false}
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s1"
      />,
    )

    // Trigger atBottomStateChange(false) to simulate user scrolling up
    act(() => {
      virtuosoMock.latestProps?.atBottomStateChange?.(false)
    })

    const button = screen.getByRole("button", { name: "chat.scrollToBottom" })
    fireEvent.click(button)

    // jump-to-bottom triggers scrollToIndex({ index: "LAST", ... })
    const calls = virtuosoMock.scrollToIndex.mock.calls
    expect(calls.length).toBeGreaterThan(0)
    expect(calls[calls.length - 1]?.[0]).toMatchObject({ index: "LAST", align: "end" })
  })

  test("forces auto-follow scroll when a new user message arrives", () => {
    const { rerender } = render(
      <MessageList
        messages={[baseMessage({ role: "assistant", content: "old", dbId: 1 })]}
        loading={false}
        agents={[]}
        hasMore={false}
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s1"
      />,
    )

    virtuosoMock.scrollToIndex.mockClear()

    rerender(
      <MessageList
        messages={[
          baseMessage({ role: "assistant", content: "old", dbId: 1 }),
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

    // forceFollowKey effect should scroll to the new user message with align: "start"
    expect(virtuosoMock.scrollToIndex).toHaveBeenCalled()
    const calls = virtuosoMock.scrollToIndex.mock.calls
    expect(calls[calls.length - 1]?.[0]).toMatchObject({
      align: "start",
      behavior: "smooth",
    })
  })

  test("shifts firstItemIndex down when older messages are prepended", () => {
    const initialMessages = [
      baseMessage({ role: "user", content: "msg-2", dbId: 2 }),
      baseMessage({ role: "assistant", content: "msg-3", dbId: 3 }),
    ]
    const { rerender } = render(
      <MessageList
        messages={initialMessages}
        loading={false}
        agents={[]}
        hasMore
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s1"
      />,
    )

    const before = virtuosoMock.latestProps?.firstItemIndex
    expect(typeof before).toBe("number")

    // Prepend two older messages — preserve the same Message object references
    // for the existing tail so the tail-equal check identifies it as a prepend.
    rerender(
      <MessageList
        messages={[
          baseMessage({ role: "user", content: "msg-0", dbId: 0 }),
          baseMessage({ role: "assistant", content: "msg-1", dbId: 1 }),
          ...initialMessages,
        ]}
        loading={false}
        agents={[]}
        hasMore
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s1"
      />,
    )

    const after = virtuosoMock.latestProps?.firstItemIndex
    expect(after).toBe((before ?? 0) - 2)
  })
})
