// @vitest-environment jsdom

import type { ReactNode } from "react"
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react"

import type { Message } from "@/types/chat"
import QuickChatMessages from "./QuickChatMessages"

const rafSpy = vi.spyOn(window, "requestAnimationFrame").mockImplementation(
  (cb: FrameRequestCallback) => {
    cb(0)
    return 0
  },
)
vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => {})

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

vi.mock("@/components/common/MarkdownRenderer", () => ({
  default: ({ content }: { content: string }) => <div>{content}</div>,
}))

vi.mock("@/components/ui/tooltip", () => ({
  IconTip: ({ children }: { children: ReactNode }) => children,
}))

beforeEach(() => {
  rafSpy.mockClear()
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

  test("scrolling near top triggers onLoadMore", () => {
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

    const el = getScroller()
    patchScrollMetrics(el, { scrollHeight: 2000, clientHeight: 600, scrollTop: 50 })
    act(() => {
      fireEvent.scroll(el)
    })
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

  test("shows the jump-to-bottom button when not at bottom and triggers scrollTo on click", () => {
    const scrollToSpy = vi.fn()
    Element.prototype.scrollTo = scrollToSpy

    render(
      <QuickChatMessages
        messages={[baseMessage({ role: "assistant", content: "streaming", dbId: 1 })]}
        loading
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

  test("forces a scroll when a new user message arrives", () => {
    const scrollIntoViewSpy = vi.fn()
    Element.prototype.scrollIntoView = scrollIntoViewSpy

    const { rerender } = render(
      <QuickChatMessages
        messages={[baseMessage({ role: "assistant", content: "old", dbId: 1 })]}
        loading={false}
        sessionId="s1"
      />,
    )

    scrollIntoViewSpy.mockClear()

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

    expect(scrollIntoViewSpy).toHaveBeenCalled()
    expect(scrollIntoViewSpy.mock.calls[scrollIntoViewSpy.mock.calls.length - 1]?.[0]).toMatchObject({
      block: "start",
      behavior: "smooth",
    })
  })
})
