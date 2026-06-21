// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, test, vi } from "vitest"
import type { ReactNode } from "react"
import type { ContentBlock, Message, ToolCall } from "@/types/chat"
import { AssistantContentBlocks } from "./MessageContent"

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: () => {} },
  useTranslation: () => ({
    t: (key: string, values?: Record<string, unknown>) => {
      if (key === "executionStatus.processed.completed") return "已处理"
      if (key === "executionStatus.tool.group.failedCount") return `${values?.count} failed`
      return key
    },
  }),
}))

vi.mock("@/components/common/MarkdownRenderer", () => ({
  default: ({ content }: { content: string }) => <div data-testid="markdown">{content}</div>,
  MarkdownLink: ({ href, children }: { href?: string; children: ReactNode }) => (
    <a href={href}>{children}</a>
  ),
}))

vi.mock("./ThinkingBlock", () => ({
  default: ({ content }: { content: string }) => <div data-testid="thinking-block">{content}</div>,
}))

vi.mock("./ToolCallBlock", () => ({
  default: ({ tool }: { tool: ToolCall }) => (
    <div data-testid="tool-block">{`${tool.name}:${tool.callId}`}</div>
  ),
}))

vi.mock("./ToolCallGroup", () => ({
  default: ({ tools }: { tools: ToolCall[] }) => (
    <div data-testid="tool-group">{tools.map((tool) => tool.callId).join(",")}</div>
  ),
}))

vi.mock("./TaskBlock", () => ({
  default: ({ tool }: { tool: ToolCall }) => <div data-testid="task-block">{tool.callId}</div>,
}))

vi.mock("@/components/chat/SubagentGroup", () => ({
  default: () => <div data-testid="subagent-group" />,
}))

vi.mock("@/components/chat/SubagentBlock", () => ({
  default: () => <div data-testid="subagent-block" />,
}))

vi.mock("@/components/chat/SkillProgressBlock", () => ({
  default: ({ tool }: { tool: ToolCall }) => <div data-testid="skill-block">{tool.callId}</div>,
}))

vi.mock("./PlanResultBlocks", () => ({
  AskUserQuestionResult: () => <div data-testid="ask-user-result" />,
  SubmitPlanResult: () => <div data-testid="submit-plan-result" />,
}))

afterEach(() => {
  cleanup()
})

function tool(callId: string, name = "read", result = "ok"): ToolCall {
  return {
    callId,
    name,
    arguments: "{}",
    result,
  }
}

function renderContentBlocks(
  contentBlocks: ContentBlock[],
  props: Partial<{ loading: boolean; isLast: boolean; displayMode: "bubble" | "timeline" }> = {},
) {
  const msg: Message = {
    role: "assistant",
    content: "",
    contentBlocks,
  }

  return render(
    <AssistantContentBlocks
      msg={msg}
      loading={props.loading ?? false}
      isLast={props.isLast ?? false}
      displayMode={props.displayMode ?? "bubble"}
    />,
  )
}

describe("AssistantContentBlocks processed grouping", () => {
  test("does not wrap a single thinking block", () => {
    renderContentBlocks([{ type: "thinking", content: "only thinking" }])

    expect(screen.queryByRole("button", { name: /已处理/ })).toBeNull()
    expect(screen.getByTestId("thinking-block").textContent).toBe("only thinking")
  })

  test("does not wrap a single tool block", () => {
    renderContentBlocks([{ type: "tool_call", tool: tool("call-1") }])

    expect(screen.queryByRole("button", { name: /已处理/ })).toBeNull()
    expect(screen.getByTestId("tool-block").textContent).toBe("read:call-1")
  })

  test("groups consecutive tools before processed folding", () => {
    renderContentBlocks([
      { type: "tool_call", tool: tool("call-1") },
      { type: "tool_call", tool: tool("call-2") },
    ])

    expect(screen.queryByRole("button", { name: /已处理/ })).toBeNull()
    expect(screen.getByTestId("tool-group").textContent).toBe("call-1,call-2")
    expect(screen.queryByTestId("tool-block")).toBeNull()
  })

  test("folds multiple completed process units after text arrives", () => {
    renderContentBlocks([
      { type: "thinking", content: "first thought" },
      { type: "tool_call", tool: tool("call-1") },
      { type: "tool_call", tool: tool("call-2") },
      { type: "thinking", content: "second thought" },
      { type: "text", content: "visible answer" },
    ])

    const processed = screen.getByRole("button", { name: /已处理/ })
    expect(processed.getAttribute("aria-expanded")).toBe("false")
    expect(screen.queryByTestId("thinking-block")).toBeNull()
    expect(screen.queryByTestId("tool-group")).toBeNull()
    expect(screen.getByTestId("markdown").textContent).toBe("visible answer")

    fireEvent.click(processed)

    expect(processed.getAttribute("aria-expanded")).toBe("true")
    expect(screen.getAllByTestId("thinking-block")).toHaveLength(2)
    expect(screen.getByTestId("tool-group").textContent).toBe("call-1,call-2")
  })

  test("text blocks break processed folding", () => {
    renderContentBlocks([
      { type: "thinking", content: "before text" },
      { type: "text", content: "visible answer" },
      { type: "tool_call", tool: tool("call-1") },
    ])

    expect(screen.queryByRole("button", { name: /已处理/ })).toBeNull()
    expect(screen.getByTestId("thinking-block").textContent).toBe("before text")
    expect(screen.getByTestId("markdown").textContent).toBe("visible answer")
    expect(screen.getByTestId("tool-block").textContent).toBe("read:call-1")
  })

  test("keeps completed process units visible while streaming before text arrives", () => {
    renderContentBlocks(
      [
        { type: "thinking", content: "first thought" },
        { type: "tool_call", tool: tool("call-1") },
        { type: "tool_call", tool: tool("call-2") },
        { type: "thinking", content: "second thought" },
      ],
      { loading: true, isLast: true },
    )

    // No text_delta has arrived yet, so completing the tools/thinking should
    // not replace the visible steps with an 已处理 header.
    expect(screen.queryByRole("button", { name: /已处理/ })).toBeNull()
    expect(screen.getAllByTestId("thinking-block")).toHaveLength(2)
    expect(screen.getByTestId("tool-group").textContent).toBe("call-1,call-2")
  })

  test("folds the completed prefix while streaming once text arrives", () => {
    renderContentBlocks(
      [
        { type: "thinking", content: "first thought" },
        { type: "tool_call", tool: tool("call-1") },
        { type: "tool_call", tool: tool("call-2") },
        { type: "thinking", content: "second thought" },
        { type: "text", content: "partial answer" },
      ],
      { loading: true, isLast: true },
    )

    const processed = screen.getByRole("button", { name: /已处理/ })
    expect(processed.getAttribute("aria-expanded")).toBe("false")
    expect(screen.getByTestId("markdown").textContent).toBe("partial answer")
    expect(screen.queryByTestId("tool-group")).toBeNull()

    fireEvent.click(processed)
    expect(screen.getByTestId("tool-group").textContent).toBe("call-1,call-2")
    expect(screen.getAllByTestId("thinking-block")).toHaveLength(2)
  })

  test("does not fold a completed run while the message is flagged streaming without text", () => {
    // Mirrors an abnormally interrupted turn: `loading` stays stuck true after
    // the stream_end was missed, but every step has already finished. Folding
    // still waits for assistant text so completed tool events do not cause a
    // one-frame collapse flash.
    renderContentBlocks(
      [
        { type: "thinking", content: "first thought" },
        { type: "tool_call", tool: tool("call-1") },
        { type: "tool_call", tool: tool("call-2") },
        { type: "tool_call", tool: tool("call-3") },
      ],
      { loading: true, isLast: true },
    )

    expect(screen.queryByRole("button", { name: /已处理/ })).toBeNull()
    expect(screen.getByTestId("thinking-block").textContent).toBe("first thought")
    expect(screen.getByTestId("tool-group").textContent).toBe("call-1,call-2,call-3")
  })

  test("folds a single completed tool while streaming and keeps the live text tail visible", () => {
    // Exercises the single-tool gating change (no longer gated on the
    // whole-message streaming flag) AND that a streaming trailing text block —
    // the live answer — never folds.
    renderContentBlocks(
      [
        { type: "tool_call", tool: tool("call-1") },
        { type: "thinking", content: "mid thought" },
        { type: "text", content: "partial answer" },
      ],
      { loading: true, isLast: true },
    )

    // call-1 (a lone completed tool) + the thinking fold into 已处理; the text
    // stays visible as the live tail.
    const processed = screen.getByRole("button", { name: /已处理/ })
    expect(screen.getByTestId("markdown").textContent).toBe("partial answer")

    fireEvent.click(processed)
    expect(screen.getByTestId("tool-block").textContent).toBe("read:call-1")
    expect(screen.getByTestId("thinking-block").textContent).toBe("mid thought")
  })

  test("does not fold the completed prefix in timeline mode before text arrives", () => {
    renderContentBlocks(
      [
        { type: "thinking", content: "first thought" },
        { type: "tool_call", tool: tool("call-1") },
        { type: "tool_call", tool: tool("call-2") },
        { type: "thinking", content: "second thought" },
      ],
      { loading: true, isLast: true, displayMode: "timeline" },
    )

    expect(screen.queryByRole("button", { name: /已处理/ })).toBeNull()
    expect(screen.getAllByTestId("thinking-block")).toHaveLength(2)
    expect(screen.getByTestId("tool-group").textContent).toBe("call-1,call-2")
  })

  test("folds the completed prefix in timeline mode once text arrives", () => {
    renderContentBlocks(
      [
        { type: "thinking", content: "first thought" },
        { type: "tool_call", tool: tool("call-1") },
        { type: "tool_call", tool: tool("call-2") },
        { type: "thinking", content: "second thought" },
        { type: "text", content: "partial answer" },
      ],
      { loading: true, isLast: true, displayMode: "timeline" },
    )

    screen.getByRole("button", { name: /已处理/ })
    expect(screen.getByTestId("markdown").textContent).toBe("partial answer")
    expect(screen.queryByTestId("tool-group")).toBeNull()
  })
})
