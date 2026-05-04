// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, test, vi } from "vitest"
import type { ContentBlock, Message, ToolCall } from "@/types/chat"
import { AssistantContentBlocks } from "./MessageContent"

vi.mock("react-i18next", () => ({
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
  props: Partial<{ loading: boolean; isLast: boolean }> = {},
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

  test("folds multiple completed process units and mounts details only after expand", () => {
    renderContentBlocks([
      { type: "thinking", content: "first thought" },
      { type: "tool_call", tool: tool("call-1") },
      { type: "tool_call", tool: tool("call-2") },
      { type: "thinking", content: "second thought" },
    ])

    const processed = screen.getByRole("button", { name: /已处理/ })
    expect(processed.getAttribute("aria-expanded")).toBe("false")
    expect(screen.queryByTestId("thinking-block")).toBeNull()
    expect(screen.queryByTestId("tool-group")).toBeNull()

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

  test("does not fold while the current assistant message is streaming", () => {
    renderContentBlocks(
      [
        { type: "thinking", content: "first thought" },
        { type: "tool_call", tool: tool("call-1") },
        { type: "tool_call", tool: tool("call-2") },
        { type: "thinking", content: "second thought" },
      ],
      { loading: true, isLast: true },
    )

    expect(screen.queryByRole("button", { name: /已处理/ })).toBeNull()
    expect(screen.getAllByTestId("thinking-block")).toHaveLength(2)
    expect(screen.getByTestId("tool-group").textContent).toBe("call-1,call-2")
  })
})
