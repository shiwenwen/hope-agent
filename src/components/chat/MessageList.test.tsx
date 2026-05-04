// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react"

import type { Message } from "@/types/chat"
import MessageList from "./MessageList"
import type { AskUserQuestionGroup } from "./ask-user/AskUserQuestionBlock"
import type { PlanCardData } from "./plan-mode/PlanCardBlock"

const originalScrollIntoView = Object.getOwnPropertyDescriptor(
  Element.prototype,
  "scrollIntoView",
)
const originalScrollTo = Object.getOwnPropertyDescriptor(Element.prototype, "scrollTo")

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

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
  vi.spyOn(window, "requestAnimationFrame").mockImplementation(
    (cb: FrameRequestCallback) => {
      cb(0)
      return 0
    },
  )
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => {})
  installElementMethod("scrollIntoView")
  installElementMethod("scrollTo")
})

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
  restoreElementMethod("scrollIntoView", originalScrollIntoView)
  restoreElementMethod("scrollTo", originalScrollTo)
})

function installElementMethod(name: "scrollIntoView" | "scrollTo") {
  Object.defineProperty(Element.prototype, name, {
    configurable: true,
    writable: true,
    value: () => {},
  })
}

function restoreElementMethod(
  name: "scrollIntoView" | "scrollTo",
  descriptor: PropertyDescriptor | undefined,
) {
  if (descriptor) {
    Object.defineProperty(Element.prototype, name, descriptor)
  } else {
    delete (Element.prototype as Partial<Record<"scrollIntoView" | "scrollTo", unknown>>)[name]
  }
}

function baseMessage(patch: Partial<Message>): Message {
  return {
    role: "assistant",
    content: "",
    timestamp: "2026-04-26T00:00:00.000Z",
    ...patch,
  } as Message
}

function patchScrollMetrics(
  container: HTMLElement,
  metrics: { scrollHeight: number; clientHeight: number; scrollTop?: number },
) {
  Object.defineProperty(container, "scrollHeight", {
    configurable: true,
    get: () => metrics.scrollHeight,
  })
  Object.defineProperty(container, "clientHeight", {
    configurable: true,
    get: () => metrics.clientHeight,
  })
  if (metrics.scrollTop !== undefined) {
    container.scrollTop = metrics.scrollTop
  }
}

function getScroller(): HTMLElement {
  const el = document.querySelector<HTMLElement>(".overflow-y-auto")
  if (!el) throw new Error("scroll container not found")
  return el
}

function makeMessages(count: number, prefix: string): Message[] {
  return Array.from({ length: count }, (_, i) =>
    baseMessage({
      role: i % 2 === 0 ? "user" : "assistant",
      content: `${prefix}-${i}`,
      dbId: i + 1,
      timestamp: `2026-04-26T00:${String(Math.floor(i / 60)).padStart(2, "0")}:${String(
        i % 60,
      ).padStart(2, "0")}.000Z`,
    }),
  )
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

  test("renders LoadMoreRow when hasMore is true and click triggers onLoadMore", () => {
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

  test("scrolling near top triggers onLoadMore when hasMore", () => {
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

    const el = getScroller()
    patchScrollMetrics(el, { scrollHeight: 2000, clientHeight: 600, scrollTop: 50 })
    act(() => {
      fireEvent.scroll(el)
    })
    expect(onLoadMore).toHaveBeenCalledTimes(1)
  })

  test("scrolling near top is a no-op while loadingMore", () => {
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

    const el = getScroller()
    patchScrollMetrics(el, { scrollHeight: 2000, clientHeight: 600, scrollTop: 50 })
    act(() => {
      fireEvent.scroll(el)
    })
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
    const scrollIntoViewSpy = vi
      .spyOn(Element.prototype, "scrollIntoView")
      .mockImplementation(() => {})

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

    expect(scrollIntoViewSpy).toHaveBeenCalled()
    expect(scrollIntoViewSpy.mock.calls[0]?.[0]).toMatchObject({ block: "center" })
    expect(onScrollTargetHandled).toHaveBeenCalledTimes(1)
  })

  test("shows the jump-to-bottom button while loading and not at bottom, and clicking calls scrollTo", () => {
    const scrollToSpy = vi.spyOn(Element.prototype, "scrollTo").mockImplementation(() => {})

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

    const el = getScroller()
    patchScrollMetrics(el, { scrollHeight: 2000, clientHeight: 600, scrollTop: 800 })
    act(() => {
      fireEvent.scroll(el)
    })

    const button = screen.getByRole("button", { name: "chat.scrollToBottom" })
    fireEvent.click(button)

    expect(scrollToSpy).toHaveBeenCalled()
    expect(scrollToSpy.mock.calls[0]?.[0]).toMatchObject({ behavior: "smooth" })
  })

  test("forces auto-follow scroll when a new user message arrives", () => {
    const scrollIntoViewSpy = vi
      .spyOn(Element.prototype, "scrollIntoView")
      .mockImplementation(() => {})

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

    scrollIntoViewSpy.mockClear()

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

    expect(scrollIntoViewSpy).toHaveBeenCalled()
    expect(scrollIntoViewSpy.mock.calls[scrollIntoViewSpy.mock.calls.length - 1]?.[0]).toMatchObject({
      block: "start",
      behavior: "smooth",
    })
  })

  test("does not force-scroll to the last user message when switching sessions", () => {
    const scrollIntoViewSpy = vi
      .spyOn(Element.prototype, "scrollIntoView")
      .mockImplementation(() => {})

    const { rerender } = render(
      <MessageList
        messages={[
          baseMessage({ role: "user", content: "session one question", dbId: 1 }),
          baseMessage({ role: "assistant", content: "session one answer", dbId: 2 }),
        ]}
        loading={false}
        agents={[]}
        hasMore={false}
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s1"
      />,
    )

    scrollIntoViewSpy.mockClear()

    rerender(
      <MessageList
        messages={[
          baseMessage({ role: "user", content: "session two question", dbId: 11 }),
          baseMessage({ role: "assistant", content: "session two answer", dbId: 12 }),
        ]}
        loading={false}
        agents={[]}
        hasMore={false}
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s2"
      />,
    )

    expect(scrollIntoViewSpy).not.toHaveBeenCalled()
  })

  test("resets the rendered window when the loaded message set shrinks", () => {
    const longMessages = makeMessages(231, "long")
    const shortMessages = makeMessages(10, "short")
    const { rerender } = render(
      <MessageList
        messages={longMessages}
        loading={false}
        agents={[]}
        hasMore={false}
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s1"
      />,
    )

    const el = getScroller()
    patchScrollMetrics(el, { scrollHeight: 2000, clientHeight: 600, scrollTop: 1400 })
    act(() => {
      fireEvent.scroll(el)
    })

    rerender(
      <MessageList
        messages={shortMessages}
        loading={false}
        agents={[]}
        hasMore={false}
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s1"
      />,
    )

    expect(screen.getByText("short-0")).toBeTruthy()
    expect(screen.getByText("short-9")).toBeTruthy()
  })
})
