// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, test, vi } from "vitest"
import type { ReactNode } from "react"
import type { ContentBlock, Message, ToolCall } from "@/types/chat"
import { AssistantContentBlocks } from "./MessageContent"
import { subscribeChatFocus } from "../chatFocus"

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
  default: ({ tool, labelOverride }: { tool: ToolCall; labelOverride?: string }) => (
    <div data-testid="tool-block" data-label={labelOverride} data-result={tool.result}>
      {`${tool.name}:${tool.callId}`}
    </div>
  ),
}))

vi.mock("./ToolCallGroup", () => ({
  default: ({ tools }: { tools: ToolCall[] }) => (
    <div data-testid="tool-group">{tools.map((tool) => tool.callId).join(",")}</div>
  ),
}))

vi.mock("./ScheduleEntityCard", () => ({
  default: ({ metadata }: { metadata: { entityId: string } }) => (
    <div data-testid="schedule-card">{metadata.entityId}</div>
  ),
}))

vi.mock("./RuntimeControlActivityGroup", () => ({
  default: ({ tools }: { tools: ToolCall[] }) => (
    <div data-testid="runtime-control-group">{tools.map((tool) => tool.callId).join(",")}</div>
  ),
}))

vi.mock("./TaskBlock", () => ({
  default: ({ tool }: { tool: ToolCall }) => <div data-testid="task-block">{tool.callId}</div>,
}))

vi.mock("@/components/chat/subagent/SubagentChips", () => ({
  default: ({ items }: { items: unknown[] }) => (
    <div data-testid="subagent-chip-row" data-count={items.length} />
  ),
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
  test.each(["bubble", "timeline"] as const)(
    "keeps cross-session receipts visible and navigates to the delivered message in %s mode",
    (displayMode) => {
      const onFocus = vi.fn()
      const unsubscribe = subscribeChatFocus(onFocus)
      try {
        const sent = tool("sent", "sessions_send", "The target agent is running")
        sent.metadata = {
          kind: "session_message",
          sessionId: "destination",
          sessionTitle: "Destination conversation",
          messageId: 42,
          turnId: "turn-1",
        }
        renderContentBlocks(
          [
            { type: "tool_call", tool: tool("before") },
            { type: "tool_call", tool: sent },
            { type: "tool_call", tool: tool("after") },
            { type: "text", content: "Done" },
          ],
          { displayMode },
        )
        fireEvent.click(screen.getByRole("button", { name: "chat.crossSession.sentTo" }))
        expect(onFocus).toHaveBeenCalledWith({ sessionId: "destination", targetMessageId: 42 })
        expect(screen.queryByRole("button", { name: /已处理/ })).toBeNull()
        expect(screen.queryByTestId("tool-group")).toBeNull()
      } finally {
        unsubscribe()
      }
    },
  )

  test("keeps the admitted receipt navigable while exposing a post-commit Stop in details", () => {
    const stopped = tool(
      "stopped",
      "sessions_send",
      "Cross-session turn was stopped while its message was persisted",
    )
    stopped.metadata = {
      kind: "session_message",
      sessionId: "destination",
      messageId: 42,
      turnId: "stopped-turn",
    }
    const onFocus = vi.fn()
    const unsubscribe = subscribeChatFocus(onFocus)
    try {
      renderContentBlocks([{ type: "tool_call", tool: stopped }])
      fireEvent.click(screen.getByRole("button", { name: "chat.crossSession.sentTo" }))
      expect(onFocus).toHaveBeenCalledWith({ sessionId: "destination", targetMessageId: 42 })
      fireEvent.click(screen.getByRole("button", { name: "chat.crossSession.details" }))
      expect(screen.getByTestId("tool-block").getAttribute("data-result")).toBe(stopped.result)
    } finally {
      unsubscribe()
    }
  })

  test.each([undefined, "Refusing cross-session messaging", "Tool error: target busy"])(
    "does not claim delivery without a backend receipt (%s)",
    (result) => {
      const pending = tool("send", "sessions_send")
      pending.arguments = JSON.stringify({ session_id: "destination", message: "hello" })
      pending.result = result
      renderContentBlocks([{ type: "tool_call", tool: pending }])
      expect(screen.queryByRole("button", { name: "chat.crossSession.sentTo" })).toBeNull()
      expect(screen.getByTestId("tool-block").textContent).toBe("sessions_send:send")
      if (result === "Refusing cross-session messaging") {
        expect(screen.getByTestId("tool-block").getAttribute("data-label")).toBe(
          "tools.sessions_send",
        )
      }
    },
  )

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

  test("keeps a scheduled-task card visible outside a collapsed processed group", () => {
    const cronTool = tool("cron-create", "manage_cron")
    cronTool.metadata = {
      kind: "schedule_entity",
      entityType: "cronTask",
      entityId: "job-1",
      title: "Smoke task",
    }
    renderContentBlocks([
      { type: "thinking", content: "create it" },
      { type: "tool_call", tool: cronTool },
      { type: "text", content: "done" },
    ])

    expect(screen.getByRole("button", { name: /已处理/ }).getAttribute("aria-expanded")).toBe(
      "false",
    )
    expect(screen.getByTestId("schedule-card").textContent).toBe("job-1")
    expect(screen.queryByTestId("tool-block")).toBeNull()
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

function subagentBlock(callId: string, args: object, result?: string): ContentBlock {
  return {
    type: "tool_call",
    tool: { callId, name: "subagent", arguments: JSON.stringify(args), result },
  }
}

function teamResumeBlock(callId: string, disposition: "resumed" | "no_op"): ContentBlock {
  return {
    type: "tool_call",
    tool: {
      callId,
      name: "team",
      arguments: JSON.stringify({ action: "resume", team_id: "team-1" }),
      result: JSON.stringify({
        status: disposition === "resumed" ? "resumed" : "already_complete",
        teamStatus: disposition === "resumed" ? "active" : "paused",
        disposition,
        resumedMemberCount: disposition === "resumed" ? 1 : 0,
        failedMemberCount: 0,
        failures: [],
      }),
    },
  }
}

describe("AssistantContentBlocks subagent chips", () => {
  test("renders a single spawn as one chip row with one item", () => {
    renderContentBlocks([
      subagentBlock(
        "c1",
        { action: "spawn", agent_id: "ha-main", task: "do x" },
        JSON.stringify({ run_id: "r1" }),
      ),
    ])

    expect(screen.getByTestId("subagent-chip-row").getAttribute("data-count")).toBe("1")
    expect(screen.queryByTestId("tool-block")).toBeNull()
  })

  test("merges consecutive spawn + batch_spawn into a single chip row", () => {
    renderContentBlocks([
      subagentBlock(
        "c1",
        { action: "spawn", agent_id: "a", task: "t1" },
        JSON.stringify({ run_id: "r1" }),
      ),
      subagentBlock(
        "c2",
        { action: "batch_spawn", tasks: [{ task: "t2" }, { task: "t3" }] },
        JSON.stringify({
          runs: [
            { status: "spawned", run_id: "r2" },
            { status: "spawned", run_id: "r3" },
          ],
        }),
      ),
    ])

    const rows = screen.getAllByTestId("subagent-chip-row")
    expect(rows).toHaveLength(1)
    expect(rows[0].getAttribute("data-count")).toBe("3")
  })

  test("shows a pending chip for an in-flight spawn with no result yet", () => {
    renderContentBlocks([
      subagentBlock("c1", { action: "spawn_and_wait", agent_id: "a", task: "t" }),
    ])

    expect(screen.getByTestId("subagent-chip-row").getAttribute("data-count")).toBe("1")
  })

  test("renders a non-spawn subagent action as a runtime activity", () => {
    renderContentBlocks([
      subagentBlock("c1", { action: "check", run_id: "r1" }, JSON.stringify({ status: "running" })),
    ])

    expect(screen.queryByTestId("subagent-chip-row")).toBeNull()
    expect(screen.getByTestId("runtime-control-group").textContent).toBe("c1")
    expect(screen.queryByTestId("tool-block")).toBeNull()
  })

  test("does not duplicate send continuation as a spawn chip", () => {
    renderContentBlocks([
      subagentBlock(
        "c1",
        { action: "send", thread_id: "thread-1", message: "continue" },
        JSON.stringify({ run_id: "r2", disposition: "resumed" }),
      ),
    ])

    expect(screen.queryByTestId("subagent-chip-row")).toBeNull()
    expect(screen.getByTestId("runtime-control-group").textContent).toBe("c1")
  })

  test("keeps spawn chips and following control activity separate", () => {
    renderContentBlocks([
      subagentBlock(
        "spawn",
        { action: "spawn", agent_id: "a", task: "research" },
        JSON.stringify({ run_id: "r1" }),
      ),
      subagentBlock(
        "send",
        { action: "send", thread_id: "thread-1", message: "adjust" },
        JSON.stringify({ run_id: "r1", disposition: "steered" }),
      ),
    ])

    expect(screen.getByTestId("subagent-chip-row").getAttribute("data-count")).toBe("1")
    expect(screen.getByTestId("runtime-control-group").textContent).toBe("send")
  })
})

describe("AssistantContentBlocks runtime activity grouping", () => {
  test("merges consecutive controls of the same semantic action", () => {
    renderContentBlocks([
      subagentBlock("send-1", { action: "steer", run_id: "r1", message: "a" }, "{}"),
      subagentBlock("send-2", { action: "send", run_id: "r2", message: "b" }, "{}"),
    ])

    const groups = screen.getAllByTestId("runtime-control-group")
    expect(groups).toHaveLength(1)
    expect(groups[0].textContent).toBe("send-1,send-2")
  })

  test("assistant text is a hard aggregation boundary", () => {
    renderContentBlocks([
      subagentBlock("send-1", { action: "steer", run_id: "r1", message: "a" }, "{}"),
      { type: "text", content: "direction changed" },
      subagentBlock("send-2", { action: "send", run_id: "r2", message: "b" }, "{}"),
    ])

    expect(screen.getAllByTestId("runtime-control-group")).toHaveLength(2)
  })

  test("different control actions preserve their order as separate groups", () => {
    renderContentBlocks([
      subagentBlock("send", { action: "steer", run_id: "r1", message: "a" }, "{}"),
      subagentBlock("kill", { action: "kill", run_id: "r2" }, "signal accepted"),
    ])

    expect(screen.getAllByTestId("runtime-control-group").map((node) => node.textContent)).toEqual([
      "send",
      "kill",
    ])
  })

  test.each([
    {
      name: "resumed then no-op",
      blocks: [teamResumeBlock("resumed", "resumed"), teamResumeBlock("no-op", "no_op")],
      expected: ["resumed", "no-op"],
    },
    {
      name: "no-op then resumed",
      blocks: [teamResumeBlock("no-op", "no_op"), teamResumeBlock("resumed", "resumed")],
      expected: ["no-op", "resumed"],
    },
  ])("does not merge adjacent Team resume and no-op results: $name", ({ blocks, expected }) => {
    renderContentBlocks(blocks)

    expect(screen.getAllByTestId("runtime-control-group").map((node) => node.textContent)).toEqual(
      expected,
    )
  })

  test("an ordinary tool before a control does not swallow the activity", () => {
    renderContentBlocks([
      { type: "tool_call", tool: tool("read-1") },
      {
        type: "tool_call",
        tool: {
          ...tool(
            "cancel-1",
            "runtime_cancel",
            JSON.stringify({ accepted: true, status: "requested" }),
          ),
          arguments: JSON.stringify({ kind: "async_job", id: "job-1" }),
        },
      },
    ])

    expect(screen.getByTestId("tool-block").textContent).toBe("read:read-1")
    expect(screen.getByTestId("runtime-control-group").textContent).toBe("cancel-1")
  })

  test("routes native process kill through the process-close activity", () => {
    renderContentBlocks([
      {
        type: "tool_call",
        tool: {
          ...tool("process-kill", "process", "Terminated session proc-1."),
          arguments: JSON.stringify({ action: "kill", session_id: "proc-1" }),
        },
      },
    ])

    expect(screen.getByTestId("runtime-control-group").textContent).toBe("process-kill")
    expect(screen.queryByTestId("tool-block")).toBeNull()
  })
})
