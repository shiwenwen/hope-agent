// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"

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
  initReactI18next: { type: "3rdParty", init: () => {} },
  useTranslation: () => ({ t: (key: string) => key }),
}))

vi.mock("./MessageBubble", () => ({
  default: ({
    msg,
    executionState,
    forceExpandUserContent,
  }: {
    msg: Message
    executionState?: string | null
    forceExpandUserContent?: boolean
  }) => (
    <div
      data-testid="message-bubble"
      data-message-db-id={msg.dbId ?? ""}
      data-execution-state={executionState ?? "none"}
      data-force-expand-user-content={forceExpandUserContent ? "true" : "false"}
    >
      {msg.content}
    </div>
  ),
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

  test("centers sub-agent result messages even when persisted as user role", () => {
    render(
      <MessageList
        messages={[
          baseMessage({
            role: "user",
            content: "sub-agent result",
            dbId: 1,
            isSubagentResult: true,
          }),
        ]}
        loading={false}
        agents={[]}
        hasMore={false}
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s1"
      />,
    )

    const row = document.querySelector<HTMLElement>('[data-message-id="1"]')
    expect(row?.className).toContain("justify-items-center")
    expect(row?.className).not.toContain("justify-items-end")
  })

  test("passes execution state only to the current assistant bubble", () => {
    render(
      <MessageList
        messages={[
          baseMessage({ role: "assistant", content: "old task", dbId: 1 }),
          baseMessage({ role: "assistant", content: "current task", dbId: 2 }),
        ]}
        loading
        executionState="running"
        agents={[]}
        hasMore={false}
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s1"
      />,
    )

    const bubbles = screen.getAllByTestId("message-bubble")
    expect(bubbles[0].getAttribute("data-execution-state")).toBe("none")
    expect(bubbles[1].getAttribute("data-execution-state")).toBe("running")
  })

  test("keeps failed terminal state on the current assistant bubble after loading ends", () => {
    render(
      <MessageList
        messages={[
          baseMessage({ role: "assistant", content: "old task", dbId: 1 }),
          baseMessage({ role: "assistant", content: "failed task", dbId: 2 }),
        ]}
        loading={false}
        executionState="failed"
        agents={[]}
        hasMore={false}
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s1"
      />,
    )

    const bubbles = screen.getAllByTestId("message-bubble")
    expect(bubbles[0].getAttribute("data-execution-state")).toBe("none")
    expect(bubbles[1].getAttribute("data-execution-state")).toBe("failed")
  })

  test("collapses intermediate assistant messages in a completed turn", () => {
    render(
      <MessageList
        messages={[
          baseMessage({
            role: "user",
            content: "question",
            dbId: 1,
            timestamp: "2026-04-26T00:00:00.000Z",
          }),
          baseMessage({
            role: "assistant",
            content: "step one",
            dbId: 2,
            timestamp: "2026-04-26T00:00:03.000Z",
          }),
          baseMessage({
            role: "assistant",
            content: "step two",
            dbId: 3,
            timestamp: "2026-04-26T00:00:07.000Z",
          }),
          baseMessage({
            role: "assistant",
            content: "final answer",
            dbId: 4,
            timestamp: "2026-04-26T00:00:10.000Z",
          }),
        ]}
        loading={false}
        agents={[]}
        hasMore={false}
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s1"
      />,
    )

    expect(screen.getByText("question")).toBeTruthy()
    expect(screen.getByText("final answer")).toBeTruthy()
    expect(screen.getByText("chat.completedTurnCollapsedWithDuration")).toBeTruthy()
    expect(screen.queryByText("step one")).toBeNull()
    expect(screen.queryByText("step two")).toBeNull()
  })

  test("collapses historical assistant content blocks before the final answer", () => {
    render(
      <MessageList
        messages={[
          baseMessage({ role: "user", content: "question", dbId: 1 }),
          baseMessage({
            role: "assistant",
            content: "final answer",
            dbId: 2,
            usage: { durationMs: 10_000 },
            contentBlocks: [
              { type: "thinking", content: "thinking details", durationMs: 2000 },
              {
                type: "tool_call",
                tool: {
                  callId: "call-1",
                  name: "exec",
                  arguments: "{}",
                  result: "done",
                  durationMs: 3000,
                },
              },
              { type: "text", content: "intermediate note" },
              { type: "text", content: "final answer" },
            ],
          }),
        ]}
        loading={false}
        agents={[]}
        hasMore={false}
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s1"
      />,
    )

    expect(screen.getByText("final answer")).toBeTruthy()
    expect(screen.getByText("chat.completedTurnCollapsedWithDuration")).toBeTruthy()
    expect(screen.queryByText("intermediate note")).toBeNull()

    fireEvent.click(screen.getByRole("button", { expanded: false }))
    expect(screen.getByText("intermediate note")).toBeTruthy()
  })

  test("expands collapsed historical prefix when search targets text inside it", async () => {
    const scrolled: Element[] = []
    vi.spyOn(Element.prototype, "scrollIntoView").mockImplementation(function (this: Element) {
      scrolled.push(this)
    })

    render(
      <MessageList
        messages={[
          baseMessage({ role: "user", content: "question", dbId: 1 }),
          baseMessage({
            role: "assistant",
            content: "final answer",
            dbId: 2,
            contentBlocks: [
              { type: "thinking", content: "thinking details" },
              { type: "text", content: "needle intermediate note" },
              { type: "text", content: "final answer" },
            ],
          }),
        ]}
        loading={false}
        agents={[]}
        hasMore={false}
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s1"
        pendingScrollIntent={{ messageId: 2, highlightTerms: ["needle"] }}
      />,
    )

    await waitFor(() => {
      expect(screen.getByText("needle intermediate note")).toBeTruthy()
      expect(scrolled.length).toBeGreaterThan(0)
    })
    expect(scrolled[0]?.textContent).toContain("needle intermediate note")
  })

  test("expands and collapses completed turn details", () => {
    render(
      <MessageList
        messages={[
          baseMessage({ role: "user", content: "question", dbId: 1 }),
          baseMessage({ role: "assistant", content: "step one", dbId: 2 }),
          baseMessage({ role: "assistant", content: "final answer", dbId: 3 }),
        ]}
        loading={false}
        agents={[]}
        hasMore={false}
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s1"
      />,
    )

    const toggle = screen.getByRole("button", { expanded: false })
    fireEvent.click(toggle)
    expect(screen.getByText("step one")).toBeTruthy()

    fireEvent.click(toggle)
    expect(screen.queryByText("step one")).toBeNull()
  })

  test("does not collapse completed turns when the preference is disabled", () => {
    render(
      <MessageList
        messages={[
          baseMessage({ role: "user", content: "question", dbId: 1 }),
          baseMessage({ role: "assistant", content: "step one", dbId: 2 }),
          baseMessage({ role: "assistant", content: "final answer", dbId: 3 }),
        ]}
        loading={false}
        agents={[]}
        hasMore={false}
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s1"
        autoCollapseCompletedTurns={false}
      />,
    )

    expect(screen.queryByText("chat.completedTurnCollapsed")).toBeNull()
    expect(screen.getByText("step one")).toBeTruthy()
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

    expect(screen.getByText("chat.incognitoEmptyBody")).toBeTruthy()
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
    expect(screen.queryByText("chat.incognitoEmptyBody")).toBeNull()
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

  test("scrolls pending ask-user footer into view when it appears", () => {
    const askUserGroup: AskUserQuestionGroup = {
      requestId: "ask-1",
      questions: [],
    } as unknown as AskUserQuestionGroup
    const messages = [baseMessage({ role: "user", content: "ping", dbId: 1 })]
    const { rerender } = render(
      <MessageList
        messages={messages}
        loading={false}
        agents={[]}
        hasMore={false}
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s1"
      />,
    )

    const el = getScroller()
    const metrics = { scrollHeight: 2000, clientHeight: 600, scrollTop: 800 }
    patchScrollMetrics(el, metrics)
    act(() => {
      fireEvent.scroll(el)
    })

    metrics.scrollHeight = 2400
    rerender(
      <MessageList
        messages={messages}
        loading={false}
        agents={[]}
        hasMore={false}
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s1"
        pendingQuestionGroup={askUserGroup}
      />,
    )

    expect(screen.getByTestId("ask-user-block")).toBeTruthy()
    expect(el.scrollTop).toBe(2400)
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
        pendingScrollIntent={{ messageId: 42, highlightTerms: null }}
        onScrollTargetHandled={onScrollTargetHandled}
      />,
    )

    expect(scrollIntoViewSpy).toHaveBeenCalled()
    expect(scrollIntoViewSpy.mock.calls[0]?.[0]).toMatchObject({ block: "center" })
    expect(onScrollTargetHandled).toHaveBeenCalledTimes(1)
  })

  test("auto-expands a user message before scrolling to a search hit", async () => {
    const scrollIntoViewSpy = vi
      .spyOn(Element.prototype, "scrollIntoView")
      .mockImplementation(() => {})

    render(
      <MessageList
        messages={[
          baseMessage({ role: "assistant", content: "earlier", dbId: 41 }),
          baseMessage({
            role: "user",
            content: `first line\n${"middle\n".repeat(20)}hidden needle`,
            dbId: 42,
          }),
        ]}
        loading={false}
        agents={[]}
        hasMore={false}
        loadingMore={false}
        onLoadMore={vi.fn()}
        sessionId="s1"
        pendingScrollIntent={{ messageId: 42, highlightTerms: ["needle"] }}
      />,
    )

    await waitFor(() => {
      const target = screen.getByText(/hidden needle/).closest("[data-message-db-id='42']")
      expect(target?.getAttribute("data-force-expand-user-content")).toBe("true")
      expect(scrollIntoViewSpy).toHaveBeenCalled()
    })
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

  test("forces bottom-follow when a new user message arrives after reading history", () => {
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

    const el = getScroller()
    patchScrollMetrics(el, { scrollHeight: 2000, clientHeight: 600, scrollTop: 800 })
    act(() => {
      fireEvent.scroll(el)
    })

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

    expect(el.scrollTop).toBe(2000)
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
