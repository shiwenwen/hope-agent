import i18next from "i18next"
import { describe, expect, test } from "vitest"

import type { SubagentRun, ToolCall } from "@/types/chat"
import ar from "@/i18n/locales/ar.json"
import en from "@/i18n/locales/en.json"
import ru from "@/i18n/locales/ru.json"

import {
  getRuntimeControlActivityGroupKey,
  parseRuntimeControlActivity,
  runtimeControlPluralKey,
} from "./runtimeControlActivity"

function tool(
  name: string,
  args: Record<string, unknown>,
  result?: Record<string, unknown> | string,
  extra: Partial<ToolCall> = {},
): ToolCall {
  return {
    callId: extra.callId ?? `${name}-1`,
    name,
    arguments: JSON.stringify(args),
    result: typeof result === "string" ? result : result ? JSON.stringify(result) : undefined,
    ...extra,
  }
}

function run(partial: Partial<SubagentRun> & { runId: string }): SubagentRun {
  return {
    threadId: `thread-${partial.runId}`,
    parentSessionId: "parent",
    parentAgentId: "ha-main",
    childAgentId: "researcher",
    childSessionId: `child-${partial.runId}`,
    task: "research",
    status: "running",
    depth: 1,
    startedAt: "2026-08-05T00:00:00.000Z",
    triggerKind: "spawn",
    leaseEpoch: 1,
    deliveryKind: "parent",
    ownerKind: "parent_session",
    ownerId: "parent",
    ...partial,
  }
}

describe("runtimeControlPluralKey", () => {
  test("keeps Arabic and Russian runtime-control counts in the active locale", async () => {
    const instance = i18next.createInstance()
    await instance.init({
      fallbackLng: "en",
      resources: {
        ar: { translation: ar },
        en: { translation: en },
        ru: { translation: ru },
      },
    })

    const base = "executionStatus.runtimeControl.action.agent.close"
    for (const [language, counts] of [
      ["ar", [1, 2, 3, 5, 11]],
      ["ru", [1, 2, 3, 5]],
    ] as const) {
      await instance.changeLanguage(language)
      for (const count of counts) {
        const expected =
          language === "ar"
            ? ar.executionStatus.runtimeControl.action.agent
            : ru.executionStatus.runtimeControl.action.agent
        const template = count === 1 ? expected.close_one : expected.close_other
        expect(instance.t(runtimeControlPluralKey(base, count), { count })).toBe(
          template.replace("{{count}}", String(count)),
        )
      }
    }
  })
})

describe("parseRuntimeControlActivity", () => {
  test("keeps real subagent spawns out of the activity projection", () => {
    for (const action of ["spawn", "batch_spawn", "spawn_and_wait"]) {
      expect(parseRuntimeControlActivity(tool("subagent", { action }))).toBeNull()
    }
  })

  test("classifies send/steer delivery and observe actions as completed", () => {
    const send = parseRuntimeControlActivity(
      tool(
        "subagent",
        { action: "send", thread_id: "thread-r1", message: "adjust" },
        { run_id: "r1", disposition: "steered", delivery: "enqueued" },
      ),
    )
    const check = parseRuntimeControlActivity(
      tool("subagent", { action: "check", run_id: "r1" }, { status: "running" }),
    )

    expect(send).toMatchObject({ family: "agent", action: "message", state: "completed" })
    expect(check).toMatchObject({ family: "agent", action: "observe", state: "completed" })
  })

  test("send continuation is a resume activity and needs the live run for confirmation", () => {
    const resumed = tool(
      "subagent",
      { action: "send", thread_id: "thread-old", message: "continue" },
      { run_id: "r-new", disposition: "resumed", child_agent_id: "researcher" },
    )

    expect(parseRuntimeControlActivity(resumed)).toMatchObject({
      action: "resume",
      state: "accepted",
      targetId: "r-new",
    })
    expect(
      parseRuntimeControlActivity(resumed, { subagentRuns: [run({ runId: "r-new" })] }),
    ).toMatchObject({ action: "resume", state: "completed", targetId: "r-new" })
  })

  test("kill success remains accepted until live state confirms termination", () => {
    const kill = tool("subagent", { action: "kill", run_id: "r1" }, "signal sent")

    expect(parseRuntimeControlActivity(kill)).toMatchObject({
      family: "agent",
      action: "close",
      state: "accepted",
    })
    const completed = parseRuntimeControlActivity(kill, {
      subagentRuns: [run({ runId: "r1", status: "killed" })],
    })
    expect(completed?.state).toBe("completed")
    expect(completed?.outcome).toBeUndefined()
  })

  test("does not claim a close when the target had already completed", () => {
    const item = parseRuntimeControlActivity(
      tool("subagent", { action: "kill", run_id: "r1" }, "already terminal"),
      { subagentRuns: [run({ runId: "r1", status: "completed" })] },
    )

    expect(item).toMatchObject({ state: "completed", outcome: "already_terminal" })
  })

  test("prioritizes structured single-kill disposition and terminal evidence", () => {
    const pending = parseRuntimeControlActivity(
      tool(
        "subagent",
        { action: "kill", run_id: "r1" },
        {
          disposition: "requested",
          requested: true,
          terminal: false,
          status: "running",
          final_status: null,
        },
        { isError: true },
      ),
    )
    const terminal = parseRuntimeControlActivity(
      tool(
        "subagent",
        { action: "kill", run_id: "r2" },
        {
          disposition: "requested",
          requested: true,
          terminal: true,
          status: "killed",
          final_status: "killed",
        },
      ),
    )
    const alreadyTerminal = parseRuntimeControlActivity(
      tool(
        "subagent",
        { action: "kill", run_id: "r3" },
        {
          disposition: "already_terminal",
          requested: false,
          terminal: true,
          status: "completed",
          final_status: "completed",
        },
      ),
    )
    const refused = parseRuntimeControlActivity(
      tool(
        "subagent",
        { action: "kill", run_id: "r4" },
        {
          disposition: "refused",
          requested: false,
          terminal: false,
          status: "running",
          final_status: null,
        },
      ),
    )

    expect(pending?.state).toBe("accepted")
    const completedAfterRequest = parseRuntimeControlActivity(pending!.tool, {
      subagentRuns: [run({ runId: "r1", status: "completed" })],
    })
    expect(completedAfterRequest?.state).toBe("completed")
    expect(completedAfterRequest?.outcome).toBeUndefined()
    expect(terminal?.state).toBe("completed")
    expect(terminal?.outcome).toBeUndefined()
    expect(alreadyTerminal).toMatchObject({
      state: "completed",
      outcome: "already_terminal",
    })
    expect(refused?.state).toBe("refused")
  })

  test("uses kill-all aggregate terminal and refusal boundaries", () => {
    const aggregate = (disposition: string, terminal: boolean, refusedCount = 0) => ({
      disposition,
      requested: disposition === "requested",
      terminal,
      requested_count: disposition === "requested" ? 2 : 0,
      terminal_count: terminal ? 2 : 0,
      pending_count: terminal ? 0 : 2,
      refused_count: refusedCount,
      runs: [],
    })
    const completed = parseRuntimeControlActivity(
      tool("subagent", { action: "kill_all" }, aggregate("requested", true)),
    )
    const refused = parseRuntimeControlActivity(
      tool("subagent", { action: "kill_all" }, aggregate("refused", false, 2)),
    )
    const pending = parseRuntimeControlActivity(
      tool("subagent", { action: "kill_all" }, aggregate("requested", false)),
    )
    const mixed = parseRuntimeControlActivity(
      tool("subagent", { action: "kill_all" }, aggregate("requested", false, 2)),
    )
    const noTargets = parseRuntimeControlActivity(
      tool("subagent", { action: "kill_all" }, aggregate("no_targets", true)),
    )
    const legacyNoTargets = parseRuntimeControlActivity(
      tool("subagent", { action: "kill_all" }, aggregate("already_terminal", true)),
    )

    expect(completed?.state).toBe("completed")
    expect(refused?.state).toBe("refused")
    expect(pending?.state).toBe("accepted")
    expect(mixed).toMatchObject({
      state: "accepted",
      aggregate: {
        requestedCount: 2,
        terminalCount: 0,
        pendingCount: 2,
        refusedCount: 2,
      },
    })
    expect(noTargets).toMatchObject({
      state: "completed",
      outcome: "no_targets",
      allTargets: true,
    })
    expect(legacyNoTargets).toMatchObject({
      state: "completed",
      outcome: "already_terminal",
      allTargets: true,
    })
  })

  test("distinguishes structured refusal from execution failure", () => {
    const refused = parseRuntimeControlActivity(
      tool(
        "runtime_cancel",
        { kind: "async_job", id: "j1" },
        {
          accepted: false,
          status: "refused",
          disposition: "refused",
        },
      ),
    )
    const failed = parseRuntimeControlActivity(
      tool(
        "runtime_cancel",
        { kind: "async_job", id: "j2" },
        { accepted: false, status: "error" },
        { isError: true },
      ),
    )

    expect(refused?.state).toBe("refused")
    expect(failed?.state).toBe("failed")
  })

  test("projects native process kill conservatively without parsing result prose", () => {
    const running = parseRuntimeControlActivity(
      tool("process", { action: "kill", session_id: "proc-1" }),
    )
    const accepted = parseRuntimeControlActivity(
      tool("process", { action: "kill", session_id: "proc-1" }, "Terminated session proc-1."),
    )
    const alreadyExited = parseRuntimeControlActivity(
      tool(
        "process",
        { action: "kill", session_id: "proc-2" },
        "Session proc-2 has already exited.",
      ),
    )
    const failed = parseRuntimeControlActivity(
      tool("process", { action: "kill", session_id: "proc-3" }, "Process was not controlled", {
        isError: true,
      }),
    )

    expect(running).toMatchObject({ family: "process", action: "close", state: "running" })
    expect(accepted).toMatchObject({ family: "process", action: "close", state: "accepted" })
    expect(alreadyExited).toMatchObject({
      family: "process",
      action: "close",
      state: "accepted",
    })
    expect(alreadyExited?.outcome).toBeUndefined()
    expect(failed?.state).toBe("failed")
  })

  test("uses structured job/runtime status instead of result prose", () => {
    const requested = parseRuntimeControlActivity(
      tool(
        "job_status",
        { action: "cancel", job_id: "j1" },
        {
          action: "cancel",
          job: { job_id: "j1", status: "cancelling" },
        },
      ),
    )
    const cancelled = parseRuntimeControlActivity(
      tool(
        "job_status",
        { action: "cancel", job_id: "j2" },
        {
          action: "cancel",
          job: { job_id: "j2", status: "cancelled" },
        },
      ),
    )
    const terminal = parseRuntimeControlActivity(
      tool(
        "runtime_cancel",
        { kind: "process", id: "p1" },
        {
          accepted: true,
          disposition: "requested",
          status: "failed",
          finalStatus: "failed",
          message: "arbitrary localized prose",
        },
      ),
    )

    expect(requested?.state).toBe("accepted")
    expect(cancelled?.state).toBe("completed")
    expect(terminal).toMatchObject({
      family: "process",
      action: "close",
      state: "completed",
    })
  })

  test("runtime_cancel disposition separates requested, already-terminal, and refused", () => {
    const requested = parseRuntimeControlActivity(
      tool(
        "runtime_cancel",
        { kind: "process", id: "p1" },
        {
          accepted: true,
          disposition: "requested",
          status: "running",
          finalStatus: null,
        },
      ),
    )
    const alreadyTerminal = parseRuntimeControlActivity(
      tool(
        "runtime_cancel",
        { kind: "async_job", id: "j1" },
        {
          accepted: false,
          disposition: "already_terminal",
          status: "completed",
          finalStatus: "completed",
        },
        { isError: true },
      ),
    )
    const refused = parseRuntimeControlActivity(
      tool(
        "runtime_cancel",
        { kind: "async_job", id: "missing" },
        {
          accepted: false,
          disposition: "refused",
          status: "refused",
          reason: "not_found",
        },
      ),
    )
    const completedAfterRequest = parseRuntimeControlActivity(
      tool(
        "runtime_cancel",
        { kind: "subagent", id: "r5" },
        {
          accepted: true,
          disposition: "requested",
          status: "running",
          finalStatus: null,
        },
      ),
      { subagentRuns: [run({ runId: "r5", status: "error" })] },
    )

    expect(requested?.state).toBe("accepted")
    expect(requested?.outcome).toBeUndefined()
    expect(alreadyTerminal).toMatchObject({
      state: "completed",
      outcome: "already_terminal",
    })
    expect(refused?.state).toBe("refused")
    expect(refused?.outcome).toBeUndefined()
    expect(completedAfterRequest?.state).toBe("completed")
    expect(completedAfterRequest?.outcome).toBeUndefined()
  })

  test("keeps legacy accepted=false terminal cancellations out of refused state", () => {
    for (const [kind, status] of [
      ["async_job", "completed"],
      ["subagent", "killed"],
    ] as const) {
      const item = parseRuntimeControlActivity(
        tool("runtime_cancel", { kind, id: `${kind}-terminal` }, { accepted: false, status }),
      )

      expect(item, `${kind}:${status}`).toMatchObject({
        state: "completed",
        outcome: "already_terminal",
      })
    }
  })

  test("job cancel reserves already-terminal outcome for explicit disposition", () => {
    const cancel = (result: Record<string, unknown>) =>
      parseRuntimeControlActivity(tool("job_status", { action: "cancel", job_id: "j1" }, result))
    const requestedByTerminal = cancel({
      disposition: "requested",
      terminal: true,
      final_status: "cancelled",
      job: { status: "cancelled" },
    })
    const requestedByFinalStatus = cancel({
      disposition: "requested",
      terminal: false,
      final_status: "completed",
      job: { status: "cancelling" },
    })
    const requestedByJobSnapshot = cancel({
      disposition: "requested",
      terminal: false,
      final_status: null,
      job: { status: "failed" },
    })
    const alreadyTerminal = cancel({
      disposition: "already_terminal",
      terminal: true,
      final_status: "completed",
      job: { status: "completed" },
    })
    const refused = cancel({ disposition: "refused", terminal: false })
    const pending = cancel({
      disposition: "requested",
      terminal: false,
      final_status: null,
      job: { status: "cancelling" },
    })

    for (const item of [requestedByTerminal, requestedByFinalStatus, requestedByJobSnapshot]) {
      expect(item?.state).toBe("completed")
      expect(item?.outcome).toBeUndefined()
    }
    expect(alreadyTerminal).toMatchObject({
      state: "completed",
      outcome: "already_terminal",
    })
    expect(refused?.state).toBe("refused")
    expect(pending?.state).toBe("accepted")
  })

  test("distinguishes resumed, partial, refused, and no-op Team resume outcomes", () => {
    const resumed = parseRuntimeControlActivity(
      tool(
        "team",
        { action: "resume", team_id: "t1" },
        {
          status: "resumed",
          teamStatus: "active",
          disposition: "resumed",
          resumedMemberCount: 2,
          failedMemberCount: 0,
          failures: [],
        },
      ),
    )
    const partial = parseRuntimeControlActivity(
      tool(
        "team",
        { action: "resume", team_id: "t1" },
        {
          status: "partially_resumed",
          teamStatus: "active",
          disposition: "partial",
          resumedMemberCount: 1,
          failedMemberCount: 1,
          failures: [
            {
              memberId: "member-2",
              name: "reviewer",
              reason: "old_attempt_active",
              oldAttemptStatus: "running",
            },
          ],
        },
      ),
    )
    const refused = parseRuntimeControlActivity(
      tool(
        "team",
        { action: "resume", team_id: "t1" },
        {
          status: "paused",
          teamStatus: "paused",
          disposition: "refused",
          resumedMemberCount: 0,
          failedMemberCount: 2,
          failures: [{ name: "worker", reason: "launch_failed" }],
        },
      ),
    )
    const noOp = parseRuntimeControlActivity(
      tool(
        "team",
        { action: "resume", team_id: "t1" },
        {
          status: "already_complete",
          teamStatus: "paused",
          disposition: "no_op",
          resumedMemberCount: 0,
          failedMemberCount: 0,
          resumedMembers: [],
          failures: [],
        },
      ),
    )

    expect(resumed).toMatchObject({ state: "completed" })
    expect(partial).toMatchObject({
      state: "partial",
      resumeSummary: {
        resumedCount: 1,
        failedCount: 1,
        failures: [{ label: "reviewer", reason: "old_attempt_active", status: "running" }],
      },
    })
    expect(refused).toMatchObject({ state: "refused" })
    expect(noOp).toMatchObject({
      state: "completed",
      outcome: "no_action_needed",
      resumeSummary: { resumedCount: 0, failedCount: 0, failures: [] },
    })
  })

  test("uses Team/Workflow durable states while Cron remains accepted", () => {
    const controls = [
      tool("team", { action: "pause", team_id: "t1" }, { status: "paused" }),
      tool("team", { action: "resume", team_id: "t1" }, { status: "resumed" }),
      tool(
        "workflow",
        { action: "control", command: "pause", runId: "w1" },
        { ok: true, run: { state: "paused" } },
      ),
      tool(
        "workflow",
        { action: "control", command: "resume", runId: "w1" },
        { ok: true, run: { state: "running" } },
      ),
      tool(
        "workflow",
        { action: "control", command: "cancel", runId: "w1" },
        { ok: true, run: { state: "cancelled" } },
      ),
      tool("manage_cron", { action: "pause", id: "c1" }, "Paused scheduled task"),
    ]

    expect(controls.map((call) => parseRuntimeControlActivity(call)?.state)).toEqual([
      "completed",
      "completed",
      "completed",
      "completed",
      "completed",
      "accepted",
    ])
  })

  test("does not complete Team/Workflow controls when the structured state is missing", () => {
    const team = parseRuntimeControlActivity(
      tool("team", { action: "pause", team_id: "t1" }, { ok: true }),
    )
    const workflow = parseRuntimeControlActivity(
      tool(
        "workflow",
        { action: "control", command: "cancel", runId: "w1" },
        { ok: true, message: "Workflow run cancelled" },
      ),
    )

    expect(team?.state).toBe("accepted")
    expect(workflow?.state).toBe("accepted")
  })

  test("group keys separate actions while matching repeated targets", () => {
    const first = parseRuntimeControlActivity(
      tool("subagent", { action: "steer", run_id: "r1", message: "a" }),
    )!
    const second = parseRuntimeControlActivity(
      tool("subagent", { action: "send", run_id: "r2", message: "b" }),
    )!
    const close = parseRuntimeControlActivity(tool("subagent", { action: "kill", run_id: "r3" }))!

    expect(getRuntimeControlActivityGroupKey(first)).toBe(getRuntimeControlActivityGroupKey(second))
    expect(getRuntimeControlActivityGroupKey(first)).not.toBe(
      getRuntimeControlActivityGroupKey(close),
    )
  })

  test("group keys keep resumed and no-action-needed Team results separate", () => {
    const resumed = parseRuntimeControlActivity(
      tool(
        "team",
        { action: "resume", team_id: "t1" },
        {
          status: "resumed",
          teamStatus: "active",
          disposition: "resumed",
          resumedMemberCount: 1,
          failedMemberCount: 0,
          failures: [],
        },
      ),
    )!
    const noActionNeeded = parseRuntimeControlActivity(
      tool(
        "team",
        { action: "resume", team_id: "t1" },
        {
          status: "already_complete",
          teamStatus: "paused",
          disposition: "no_op",
          resumedMemberCount: 0,
          failedMemberCount: 0,
          failures: [],
        },
      ),
    )!

    expect(getRuntimeControlActivityGroupKey(resumed)).not.toBe(
      getRuntimeControlActivityGroupKey(noActionNeeded),
    )
  })
})
