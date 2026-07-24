import { describe, expect, test } from "vitest"
import type { SubagentRun, ToolCall } from "@/types/chat"
import { DEFAULT_AGENT_ID } from "@/types/tools"
import {
  extractSubagentChipItems,
  indexLatestRunByChildSession,
  markdownPreview,
  matchPendingRun,
} from "./subagentRunModel"

function tool(partial: Partial<ToolCall> & { callId: string }): ToolCall {
  return { name: "subagent", arguments: "{}", ...partial }
}

function run(partial: Partial<SubagentRun> & { runId: string }): SubagentRun {
  return {
    threadId: `cs-${partial.runId}`,
    parentSessionId: "p",
    parentAgentId: "pa",
    childAgentId: "a",
    childSessionId: `cs-${partial.runId}`,
    task: "t",
    status: "running",
    depth: 1,
    startedAt: "2026-07-18T00:00:00.000Z",
    triggerKind: "spawn",
    leaseEpoch: 1,
    deliveryKind: "parent",
    ownerKind: "parent_session",
    ownerId: "p",
    ...partial,
  }
}

describe("extractSubagentChipItems", () => {
  test("spawn with a result → one resolved item", () => {
    const items = extractSubagentChipItems(
      tool({
        callId: "c1",
        arguments: JSON.stringify({ action: "spawn", agent_id: "a", task: "do x", label: "L" }),
        result: JSON.stringify({ run_id: "r1" }),
      }),
    )
    expect(items).toEqual([
      { kind: "resolved", key: "c1", runId: "r1", agentId: "a", task: "do x", label: "L" },
    ])
  })

  test("spawn_and_wait without a result → one pending item keyed on callId", () => {
    const items = extractSubagentChipItems(
      tool({
        callId: "c1",
        startedAtMs: 1000,
        arguments: JSON.stringify({ action: "spawn_and_wait", agent_id: "a", task: "t" }),
      }),
    )
    expect(items).toEqual([
      { kind: "pending", key: "c1", agentId: "a", task: "t", label: undefined, startedAtMs: 1000 },
    ])
  })

  test("resume result → one resolved item for the fresh run and actual child agent", () => {
    const items = extractSubagentChipItems(
      tool({
        callId: "c-resume",
        arguments: JSON.stringify({ action: "resume", run_id: "r-old", task: "check again" }),
        result: JSON.stringify({ run_id: "r-new", child_agent_id: "researcher" }),
      }),
    )
    expect(items).toEqual([
      {
        kind: "resolved",
        key: "c-resume",
        runId: "r-new",
        agentId: "researcher",
        task: "check again",
        label: undefined,
      },
    ])
  })

  test("send is hidden while pending and resolves to the authoritative current attempt", () => {
    const args = { action: "send", thread_id: "child-1", message: "continue" }
    expect(
      extractSubagentChipItems(tool({ callId: "c-send", arguments: JSON.stringify(args) })),
    ).toEqual([])
    expect(
      extractSubagentChipItems(
        tool({
          callId: "c-send",
          arguments: JSON.stringify(args),
          result: JSON.stringify({ run_id: "r-current", child_agent_id: "researcher" }),
        }),
      ),
    ).toEqual([
      {
        kind: "resolved",
        key: "c-send",
        runId: "r-current",
        agentId: "researcher",
        task: "continue",
        label: undefined,
      },
    ])
  })

  test("batch_spawn keeps only spawned entries, mapped to their task defs", () => {
    const items = extractSubagentChipItems(
      tool({
        callId: "c1",
        arguments: JSON.stringify({
          action: "batch_spawn",
          tasks: [{ task: "t1", agent_id: "x" }, { task: "t2" }, { task: "t3" }],
        }),
        result: JSON.stringify({
          runs: [
            { status: "spawned", run_id: "r1" },
            { status: "failed" },
            { status: "spawned", run_id: "r3" },
          ],
        }),
      }),
    )
    expect(items).toEqual([
      { kind: "resolved", key: "c1:0", runId: "r1", agentId: "x", task: "t1", label: undefined },
      {
        kind: "resolved",
        key: "c1:2",
        runId: "r3",
        agentId: DEFAULT_AGENT_ID,
        task: "t3",
        label: undefined,
      },
    ])
  })

  test("batch_spawn without a result → one pending item per declared task", () => {
    const items = extractSubagentChipItems(
      tool({
        callId: "c1",
        startedAtMs: 5,
        arguments: JSON.stringify({
          action: "batch_spawn",
          tasks: [{ task: "t1" }, { task: "t2" }],
        }),
      }),
    )
    expect(items.map((i) => (i.kind === "pending" ? i.key : i.runId))).toEqual(["c1:0", "c1:1"])
  })

  test("non-spawn action → empty (falls back to a plain tool block)", () => {
    expect(
      extractSubagentChipItems(
        tool({ callId: "c1", arguments: JSON.stringify({ action: "check", run_id: "r1" }) }),
      ),
    ).toEqual([])
  })

  test("malformed arguments → empty", () => {
    expect(extractSubagentChipItems(tool({ callId: "c1", arguments: "not json" }))).toEqual([])
  })

  test("a pending item and its resolved item share the same key (no remount on flip)", () => {
    const args = { action: "spawn_and_wait", agent_id: "a", task: "t" }
    const pending = extractSubagentChipItems(
      tool({ callId: "c9", arguments: JSON.stringify(args) }),
    )
    const resolved = extractSubagentChipItems(
      tool({
        callId: "c9",
        arguments: JSON.stringify(args),
        result: JSON.stringify({ run_id: "r9" }),
      }),
    )
    expect(pending[0].key).toBe("c9")
    expect(resolved[0].key).toBe("c9")
  })
})

describe("indexLatestRunByChildSession", () => {
  test("keeps the newest run when resume creates multiple rows for one child session", () => {
    const runs = [
      run({ runId: "r-new", childSessionId: "child-1" }),
      run({ runId: "r-old", childSessionId: "child-1" }),
    ]
    expect(indexLatestRunByChildSession(runs).get("child-1")?.runId).toBe("r-new")
  })
})

describe("markdownPreview", () => {
  test("drops a leading fenced code block instead of previewing its source", () => {
    const result = "```python\nfrom collections import deque\n```\n实现了广度优先遍历。"
    expect(markdownPreview(result)).toBe("实现了广度优先遍历。")
  })

  test("keeps identifiers intact — underscores are never touched", () => {
    // A blanket [*_~] strip would yield "modelcampaign" / "init".
    expect(markdownPreview("统一 `model_campaign` 与 __init__ 口径")).toBe(
      "统一 model_campaign 与 __init__ 口径",
    )
  })

  test("strips balanced emphasis pairs so previews aren't littered with markers", () => {
    expect(markdownPreview("**重点**：已完成 *部分* 与 ~~废弃~~ 项")).toBe(
      "重点：已完成 部分 与 废弃 项",
    )
  })

  test("leaves an unpaired asterisk alone (e.g. a lone **kwargs)", () => {
    expect(markdownPreview("传入 **kwargs 即可")).toBe("传入 **kwargs 即可")
  })

  test("keeps ordered-list numbering (it carries meaning)", () => {
    expect(markdownPreview("1. 做了 X\n2. 做了 Y")).toBe("1. 做了 X 2. 做了 Y")
  })

  test("strips leading block markers and reduces links to their text", () => {
    expect(markdownPreview("## 概览\n- 见 [路线图](https://example.com/a.md)\n> 备注")).toBe(
      "概览 见 路线图 备注",
    )
  })

  test("an all-code result reduces to empty so callers can fall back", () => {
    expect(markdownPreview("```ts\nconst a = 1\n```")).toBe("")
  })
})

describe("matchPendingRun", () => {
  test("matches by agent + task and skips already-claimed runs", () => {
    const runs = [run({ runId: "r2", startedAt: "2026-07-18T00:00:02.000Z" }), run({ runId: "r1" })]
    const claimed = new Set<string>(["r1"])
    const match = matchPendingRun(runs, { agentId: "a", task: "t" }, undefined, claimed)
    expect(match?.runId).toBe("r2")
  })

  test("distinct pending chips claim distinct runs oldest-first", () => {
    // DESC order (newest first), same agent + task.
    const runs = [
      run({ runId: "rNew", startedAt: "2026-07-18T00:00:05.000Z" }),
      run({ runId: "rOld", startedAt: "2026-07-18T00:00:01.000Z" }),
    ]
    const claimed = new Set<string>()
    const first = matchPendingRun(runs, { agentId: "a", task: "t" }, undefined, claimed)
    expect(first?.runId).toBe("rOld")
    claimed.add(first!.runId)
    const second = matchPendingRun(runs, { agentId: "a", task: "t" }, undefined, claimed)
    expect(second?.runId).toBe("rNew")
  })

  test("ignores runs that started well before the tool call", () => {
    const runs = [run({ runId: "rOld", startedAt: "2026-07-18T00:00:00.000Z" })]
    // Tool started at 00:01:00 → the run is >15s older, so no match.
    const startedAtMs = Date.parse("2026-07-18T00:01:00.000Z")
    expect(matchPendingRun(runs, { agentId: "a", task: "t" }, startedAtMs, new Set())).toBeNull()
  })

  test("ignores runs that started well after the tool call (upper bound)", () => {
    const runs = [run({ runId: "rFuture", startedAt: "2026-07-18T00:10:00.000Z" })]
    // Tool started at 00:00:00 → the run is >60s newer, so no match.
    const startedAtMs = Date.parse("2026-07-18T00:00:00.000Z")
    expect(matchPendingRun(runs, { agentId: "a", task: "t" }, startedAtMs, new Set())).toBeNull()
  })

  test("returns null when nothing matches", () => {
    const runs = [run({ runId: "r1", childAgentId: "other" })]
    expect(matchPendingRun(runs, { agentId: "a", task: "t" }, undefined, new Set())).toBeNull()
  })
})
