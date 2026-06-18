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

  test("folds the completed prefix while streaming, leaving the live tail unfolded", () => {
    renderContentBlocks(
      [
        { type: "thinking", content: "first thought" },
        { type: "tool_call", tool: tool("call-1") },
        { type: "tool_call", tool: tool("call-2") },
        { type: "thinking", content: "second thought" },
      ],
      { loading: true, isLast: true },
    )

    // First thought + the two completed tools fold into one 已处理 group; the
    // trailing thinking block is the live tail and stays expanded.
    const processed = screen.getByRole("button", { name: /已处理/ })
    expect(processed.getAttribute("aria-expanded")).toBe("false")
    expect(screen.getByTestId("thinking-block").textContent).toBe("second thought")
    expect(screen.queryByTestId("tool-group")).toBeNull()

    fireEvent.click(processed)
    // first thought + tool group live inside the folded prefix.
    expect(screen.getByTestId("tool-group").textContent).toBe("call-1,call-2")
    expect(screen.getAllByTestId("thinking-block")).toHaveLength(2)
  })

  test("folds a fully-completed run even while the message is flagged streaming", () => {
    // Mirrors an abnormally interrupted turn: `loading` stays stuck true after
    // the stream_end was missed, but every step has already finished. Folding
    // must not depend on the loading flag — the completed steps still collapse.
    renderContentBlocks(
      [
        { type: "thinking", content: "first thought" },
        { type: "tool_call", tool: tool("call-1") },
        { type: "tool_call", tool: tool("call-2") },
        { type: "tool_call", tool: tool("call-3") },
      ],
      { loading: true, isLast: true },
    )

    // Last block is a tool_call → a non-complete `__loading__` tail unit is
    // appended, so the three tools + thinking fold into one 已处理 group.
    screen.getByRole("button", { name: /已处理/ })
    expect(screen.queryByTestId("tool-group")).toBeNull()
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

  test("folds the completed prefix in timeline mode while streaming", () => {
    renderContentBlocks(
      [
        { type: "thinking", content: "first thought" },
        { type: "tool_call", tool: tool("call-1") },
        { type: "tool_call", tool: tool("call-2") },
        { type: "thinking", content: "second thought" },
      ],
      { loading: true, isLast: true, displayMode: "timeline" },
    )

    // Same folding as bubble mode: completed prefix → one 已处理 timeline item,
    // trailing thinking stays as the live tail.
    screen.getByRole("button", { name: /已处理/ })
    expect(screen.getByTestId("thinking-block").textContent).toBe("second thought")
    expect(screen.queryByTestId("tool-group")).toBeNull()
  })
})
