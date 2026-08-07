import type { SubagentRun, ToolCall } from "@/types/chat"

import { hasToolError } from "./executionStatus"

export type RuntimeControlFamily = "agent" | "job" | "process" | "team" | "workflow" | "cron"

export type RuntimeControlAction = "observe" | "message" | "pause" | "resume" | "cancel" | "close"

export type RuntimeControlActivityState =
  | "running"
  | "accepted"
  | "completed"
  | "partial"
  | "failed"
  | "refused"

export interface RuntimeControlAggregateCounts {
  requestedCount?: number
  terminalCount?: number
  pendingCount?: number
  refusedCount?: number
}

export interface RuntimeControlFailureDetail {
  label?: string
  reason?: string
  status?: string
}

export interface RuntimeControlResumeSummary {
  resumedCount?: number
  failedCount?: number
  failures: RuntimeControlFailureDetail[]
}

export interface RuntimeControlActivityItem {
  key: string
  family: RuntimeControlFamily
  action: RuntimeControlAction
  state: RuntimeControlActivityState
  targetId?: string
  label?: string
  callId: string
  durationMs?: number
  detail?: string
  /** The target is terminal, but this control action did not confirm a new close/cancel. */
  outcome?: "already_terminal" | "no_targets" | "no_action_needed"
  /** `kill_all` is rendered as one aggregate control rather than a per-run count. */
  allTargets?: boolean
  /** Structured aggregate counts returned by `subagent(kill_all)`. */
  aggregate?: RuntimeControlAggregateCounts
  /** Structured member outcomes returned by `team(resume)`. */
  resumeSummary?: RuntimeControlResumeSummary
  tool: ToolCall
}

export interface RuntimeControlParseContext {
  /** Session-scoped live/durable runs, newest first. */
  subagentRuns?: readonly SubagentRun[]
}

/**
 * Runtime-control copy intentionally ships a generic singular/plural pair for
 * every locale. Resolve that pair explicitly instead of asking i18next for the
 * locale's full CLDR form set: otherwise languages such as Arabic and Russian
 * fall through to English when `_two`, `_few`, or `_many` is absent.
 */
export function runtimeControlPluralKey(baseKey: string, count: number): string {
  return `${baseKey}_${count === 1 ? "one" : "other"}`
}

type JsonRecord = Record<string, unknown>

const SUBAGENT_SPAWN_ACTIONS = new Set(["spawn", "batch_spawn", "spawn_and_wait"])
const SUBAGENT_OBSERVE_ACTIONS = new Set(["check", "list", "result", "wait_all"])
const SUBAGENT_TERMINAL_STATUSES = new Set([
  "completed",
  "error",
  "timeout",
  "killed",
  "interrupted",
])
const REFUSED_STATUSES = new Set([
  "refused",
  "denied",
  "forbidden",
  "unauthorized",
  "not_found",
  "not_running",
  "not_owned",
])
const REQUESTED_STATUSES = new Set([
  "accepted",
  "requested",
  "cancelling",
  "cancel_requested",
  "pause_requested",
  "resume_requested",
  "queued",
  "running",
])
const LEGACY_ALREADY_TERMINAL_STATUSES = new Set([
  "terminal",
  "completed",
  "timeout",
  "timed_out",
  "killed",
  "interrupted",
  "cancelled",
  "canceled",
])

function parseRecord(value: string | undefined): JsonRecord | null {
  if (!value) return null
  try {
    const parsed: unknown = JSON.parse(value)
    return parsed && typeof parsed === "object" && !Array.isArray(parsed)
      ? (parsed as JsonRecord)
      : null
  } catch {
    return null
  }
}

function nestedRecord(value: unknown): JsonRecord | null {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as JsonRecord) : null
}

function stringField(record: JsonRecord | null, ...keys: string[]): string | undefined {
  if (!record) return undefined
  for (const key of keys) {
    const value = record[key]
    if (typeof value === "string" && value.trim()) return value.trim()
  }
  return undefined
}

function normalizedField(record: JsonRecord | null, ...keys: string[]): string | undefined {
  return stringField(record, ...keys)?.toLowerCase()
}

function booleanField(record: JsonRecord | null, ...keys: string[]): boolean | undefined {
  if (!record) return undefined
  for (const key of keys) {
    if (typeof record[key] === "boolean") return record[key] as boolean
  }
  return undefined
}

function nonNegativeIntegerField(record: JsonRecord | null, ...keys: string[]): number | undefined {
  if (!record) return undefined
  for (const key of keys) {
    const value = record[key]
    if (typeof value === "number" && Number.isFinite(value) && value >= 0) {
      return Math.floor(value)
    }
  }
  return undefined
}

function killAllAggregate(result: JsonRecord | null): RuntimeControlAggregateCounts | undefined {
  const aggregate: RuntimeControlAggregateCounts = {
    requestedCount: nonNegativeIntegerField(result, "requested_count", "requestedCount"),
    terminalCount: nonNegativeIntegerField(result, "terminal_count", "terminalCount"),
    pendingCount: nonNegativeIntegerField(result, "pending_count", "pendingCount"),
    refusedCount: nonNegativeIntegerField(result, "refused_count", "refusedCount"),
  }
  return Object.values(aggregate).some((value) => value !== undefined) ? aggregate : undefined
}

function teamResumeSummary(result: JsonRecord | null): RuntimeControlResumeSummary | undefined {
  const rawFailures = result && Array.isArray(result.failures) ? result.failures : []
  const failures = rawFailures.map((failure): RuntimeControlFailureDetail => {
    const record = nestedRecord(failure)
    if (!record) return typeof failure === "string" ? { reason: failure } : {}
    return {
      label: stringField(record, "name", "memberId", "member_id"),
      reason: stringField(record, "reason", "message"),
      status: stringField(record, "oldAttemptStatus", "old_attempt_status", "status"),
    }
  })
  const resumedCount = nonNegativeIntegerField(result, "resumedMemberCount", "resumed_member_count")
  const failedCount =
    nonNegativeIntegerField(result, "failedMemberCount", "failed_member_count") ??
    (failures.length > 0 ? failures.length : undefined)
  if (resumedCount === undefined && failedCount === undefined && failures.length === 0) {
    return undefined
  }
  return { resumedCount, failedCount, failures }
}

function findSubagentRun(
  runs: readonly SubagentRun[] | undefined,
  id: string | undefined,
): SubagentRun | undefined {
  if (!id || !runs) return undefined
  return runs.find((run) => run.runId === id || run.threadId === id || run.childSessionId === id)
}

function subagentTargetLabel(
  args: JsonRecord,
  result: JsonRecord | null,
  run: SubagentRun | undefined,
): string | undefined {
  return (
    stringField(args, "label") ??
    run?.label ??
    stringField(result, "child_agent_id", "childAgentId") ??
    run?.childAgentId
  )
}

function baseToolState(tool: ToolCall): RuntimeControlActivityState | null {
  if (tool.result === undefined) return "running"
  if (resultWasRefused(parseRecord(tool.result))) return "refused"
  if (hasToolError(tool)) return "failed"
  return null
}

function resultWasRefused(result: JsonRecord | null): boolean {
  const status = normalizedField(result, "status", "outcome")
  const disposition = normalizedField(result, "disposition", "delivery")
  if (disposition === "already_terminal") return false
  return (
    status === "refused" ||
    disposition === "refused" ||
    (booleanField(result, "accepted") === false &&
      status !== "already_terminal" &&
      status !== "error" &&
      status !== "failed")
  )
}

function makeItem(
  tool: ToolCall,
  family: RuntimeControlFamily,
  action: RuntimeControlAction,
  state: RuntimeControlActivityState,
  extra: Partial<RuntimeControlActivityItem> = {},
): RuntimeControlActivityItem {
  return {
    key: tool.callId,
    family,
    action,
    state,
    callId: tool.callId,
    durationMs: tool.durationMs,
    tool,
    ...extra,
  }
}

function parseSubagentActivity(
  tool: ToolCall,
  args: JsonRecord,
  context: RuntimeControlParseContext,
): RuntimeControlActivityItem | null {
  const requestedAction = normalizedField(args, "action")
  if (!requestedAction || SUBAGENT_SPAWN_ACTIONS.has(requestedAction)) return null

  const recognized =
    requestedAction === "send" ||
    requestedAction === "steer" ||
    requestedAction === "resume" ||
    requestedAction === "kill" ||
    requestedAction === "kill_all" ||
    SUBAGENT_OBSERVE_ACTIONS.has(requestedAction)
  if (!recognized) return null

  const result = parseRecord(tool.result)
  const resultDisposition = normalizedField(result, "disposition")
  const isResume =
    requestedAction === "resume" ||
    (requestedAction === "send" &&
      (normalizedField(args, "mode") === "resume_only" || resultDisposition === "resumed"))
  const action: RuntimeControlAction = SUBAGENT_OBSERVE_ACTIONS.has(requestedAction)
    ? "observe"
    : requestedAction === "kill" || requestedAction === "kill_all"
      ? "close"
      : isResume
        ? "resume"
        : "message"

  const resultRunId = stringField(result, "run_id", "runId")
  const requestedTargetId = stringField(args, "run_id", "runId", "thread_id", "threadId")
  const targetId =
    action === "resume" ? (resultRunId ?? requestedTargetId) : (requestedTargetId ?? resultRunId)
  const liveRun = findSubagentRun(context.subagentRuns, targetId)
  const common = {
    targetId,
    label: subagentTargetLabel(args, result, liveRun),
    allTargets: requestedAction === "kill_all" || undefined,
    aggregate: requestedAction === "kill_all" ? killAllAggregate(result) : undefined,
  }

  if (action === "close" && result && resultDisposition) {
    const terminal = booleanField(result, "terminal")

    if (requestedAction === "kill_all") {
      // `kill_all` reports one aggregate boundary. Individual live runs must
      // not be projected as proof that the complete batch has terminated.
      if (resultDisposition === "refused") {
        return makeItem(tool, "agent", action, "refused", common)
      }
      if (resultDisposition === "no_targets") {
        return makeItem(tool, "agent", action, "completed", {
          ...common,
          outcome: "no_targets",
        })
      }
      if (terminal === true) {
        return makeItem(tool, "agent", action, "completed", {
          ...common,
          outcome: resultDisposition === "already_terminal" ? "already_terminal" : undefined,
        })
      }
      return makeItem(tool, "agent", action, "accepted", common)
    }

    if (requestedAction === "kill") {
      // The structured disposition describes whether this call requested the
      // close; the live projection may subsequently confirm a pending request.
      if (resultDisposition === "refused") {
        return makeItem(tool, "agent", action, "refused", common)
      }
      if (resultDisposition === "already_terminal") {
        return makeItem(tool, "agent", action, "completed", {
          ...common,
          outcome: "already_terminal",
        })
      }
      if (terminal === true) {
        return makeItem(tool, "agent", action, "completed", common)
      }
      if (liveRun && SUBAGENT_TERMINAL_STATUSES.has(liveRun.status)) {
        // This is a newer observation than the requested disposition. Any
        // terminal state therefore confirms completion after the request;
        // only an explicit already_terminal disposition has pre-call meaning.
        return makeItem(tool, "agent", action, "completed", common)
      }
      return makeItem(tool, "agent", action, "accepted", common)
    }
  }

  const baseState = baseToolState(tool)
  if (baseState) return makeItem(tool, "agent", action, baseState, common)
  if (resultWasRefused(result)) return makeItem(tool, "agent", action, "refused", common)

  if (action === "observe") {
    return makeItem(tool, "agent", action, "completed", common)
  }

  if (action === "message") {
    // `steered` is emitted only after the durable dispatch was recorded. The
    // child run itself may continue for a long time; this activity describes
    // message delivery, not completion of that run.
    const delivered =
      resultDisposition === "steered" ||
      ["accepted", "enqueued", "delivered"].includes(normalizedField(result, "delivery") ?? "")
    return makeItem(tool, "agent", action, delivered ? "completed" : "accepted", common)
  }

  if (action === "resume") {
    // A result handle alone says only that the request was accepted. Confirm
    // the fresh immutable attempt from the session-scoped run projection.
    return makeItem(tool, "agent", action, liveRun ? "completed" : "accepted", common)
  }

  if (action === "close" && liveRun && SUBAGENT_TERMINAL_STATUSES.has(liveRun.status)) {
    if (liveRun.status === "killed" || liveRun.status === "interrupted") {
      return makeItem(tool, "agent", action, "completed", common)
    }
    return makeItem(tool, "agent", action, "completed", {
      ...common,
      outcome: "already_terminal",
    })
  }

  // A successful kill/kill_all tool call acknowledges a cancellation signal;
  // it is not proof that every target has crossed its terminal boundary.
  return makeItem(tool, "agent", action, "accepted", common)
}

function parseJobCancelActivity(
  tool: ToolCall,
  args: JsonRecord,
): RuntimeControlActivityItem | null {
  if (normalizedField(args, "action") !== "cancel") return null
  const targetId = stringField(args, "job_id", "jobId")
  const common = { targetId }
  if (tool.result === undefined) return makeItem(tool, "job", "cancel", "running", common)

  const result = parseRecord(tool.result)
  const disposition = normalizedField(result, "disposition")
  const terminal = booleanField(result, "terminal")
  const finalStatus = normalizedField(result, "finalStatus", "final_status")
  const job = nestedRecord(result?.job)
  const targetStatus = normalizedField(job, "status")
  const jobIsTerminal = [
    "cancelled",
    "canceled",
    "completed",
    "failed",
    "interrupted",
    "timed_out",
  ].includes(targetStatus ?? "")

  if (disposition) {
    if (disposition === "already_terminal") {
      return makeItem(tool, "job", "cancel", "completed", {
        ...common,
        outcome: "already_terminal",
      })
    }
    if (disposition === "refused") {
      return makeItem(tool, "job", "cancel", "refused", common)
    }
    if (disposition === "requested" && (terminal === true || finalStatus || jobIsTerminal)) {
      return makeItem(tool, "job", "cancel", "completed", common)
    }
    return makeItem(tool, "job", "cancel", "accepted", common)
  }

  // Historical results did not distinguish a pre-call terminal target from a
  // terminal state observed after requesting cancellation. Keep their former,
  // conservative projection rather than inventing temporal evidence.
  const baseState = baseToolState(tool)
  if (baseState) return makeItem(tool, "job", "cancel", baseState, common)
  if (resultWasRefused(result)) return makeItem(tool, "job", "cancel", "refused", common)
  if (targetStatus === "cancelled" || targetStatus === "canceled") {
    return makeItem(tool, "job", "cancel", "completed", common)
  }
  if (["completed", "failed", "interrupted", "timed_out"].includes(targetStatus ?? "")) {
    return makeItem(tool, "job", "cancel", "completed", {
      ...common,
      outcome: "already_terminal",
    })
  }
  return makeItem(tool, "job", "cancel", "accepted", common)
}

function parseNativeProcessActivity(
  tool: ToolCall,
  args: JsonRecord,
): RuntimeControlActivityItem | null {
  if (normalizedField(args, "action") !== "kill") return null
  const common = { targetId: stringField(args, "session_id", "sessionId") }
  const baseState = baseToolState(tool)
  if (baseState) return makeItem(tool, "process", "close", baseState, common)

  // The native process tool currently returns prose for both a successful
  // termination and an already-exited session. Treat success only as an
  // accepted close request until the backend exposes structured state.
  return makeItem(tool, "process", "close", "accepted", common)
}

function parseRuntimeCancelActivity(
  tool: ToolCall,
  args: JsonRecord,
  context: RuntimeControlParseContext,
): RuntimeControlActivityItem | null {
  const kind = normalizedField(args, "kind")
  const targetId = stringField(args, "id")
  if (!kind || !targetId) return null
  const family: RuntimeControlFamily =
    kind === "subagent" ? "agent" : kind === "process" ? "process" : "job"
  const action: RuntimeControlAction =
    kind === "subagent" || kind === "process" ? "close" : "cancel"
  const liveRun = kind === "subagent" ? findSubagentRun(context.subagentRuns, targetId) : undefined
  const common = {
    targetId,
    label: liveRun?.label ?? liveRun?.childAgentId,
    detail: kind,
  }
  if (tool.result === undefined) return makeItem(tool, family, action, "running", common)

  const result = parseRecord(tool.result)
  const accepted = booleanField(result, "accepted")
  const disposition = normalizedField(result, "disposition")
  const status = normalizedField(result, "status", "outcome")
  const finalStatus = normalizedField(result, "finalStatus", "final_status")

  // `disposition` is the write-request outcome; `status` is only the latest
  // observed target state. Always resolve the former first.
  if (disposition === "already_terminal") {
    return makeItem(tool, family, action, "completed", {
      ...common,
      outcome: "already_terminal",
    })
  }
  if (disposition === "refused") {
    return makeItem(tool, family, action, "refused", common)
  }
  if (disposition === "requested") {
    if (finalStatus) return makeItem(tool, family, action, "completed", common)
    if (kind === "subagent" && liveRun && SUBAGENT_TERMINAL_STATUSES.has(liveRun.status)) {
      return makeItem(tool, family, action, "completed", common)
    }
    return makeItem(tool, family, action, "accepted", common)
  }

  if (!disposition && status === "already_terminal") {
    return makeItem(tool, family, action, "completed", {
      ...common,
      outcome: "already_terminal",
    })
  }

  // Compatibility for historical tool results written before disposition and
  // finalStatus were added. In that schema, accepted=false plus the target's
  // terminal status meant "nothing to cancel", not an authorization refusal.
  // Resolve it before baseToolState/resultWasRefused interpret accepted=false.
  if (accepted === false && LEGACY_ALREADY_TERMINAL_STATUSES.has(status ?? "")) {
    return makeItem(tool, family, action, "completed", {
      ...common,
      outcome: "already_terminal",
    })
  }

  const baseState = baseToolState(tool)
  if (baseState) return makeItem(tool, family, action, baseState, common)
  if (status === "error" || status === "failed") {
    return makeItem(tool, family, action, "failed", common)
  }
  if (REFUSED_STATUSES.has(status ?? "") || accepted === false) {
    return makeItem(tool, family, action, "refused", common)
  }

  if (kind === "subagent") {
    if (liveRun && SUBAGENT_TERMINAL_STATUSES.has(liveRun.status)) {
      if (liveRun.status === "killed" || liveRun.status === "interrupted") {
        return makeItem(tool, family, action, "completed", common)
      }
      return makeItem(tool, family, action, "completed", {
        ...common,
        outcome: "already_terminal",
      })
    }
    return makeItem(tool, family, action, "accepted", common)
  }

  // Structured `terminal`/`cancelled` is an explicit target-state guarantee,
  // unlike a successful tool call or a free-form message. Process cancellation
  // currently reports `killed` only after the process registry is terminal.
  if (
    status === "terminal" ||
    status === "cancelled" ||
    status === "canceled" ||
    (kind === "process" && status === "killed")
  ) {
    return makeItem(tool, family, action, "completed", common)
  }
  if (REQUESTED_STATUSES.has(status ?? "") || accepted === true) {
    return makeItem(tool, family, action, "accepted", common)
  }
  return makeItem(tool, family, action, "accepted", common)
}

function parseAcceptedControl(
  tool: ToolCall,
  args: JsonRecord,
  family: "team" | "cron",
): RuntimeControlActivityItem | null {
  const action = normalizedField(args, "action")
  if (action !== "pause" && action !== "resume") return null
  const targetId = stringField(args, family === "team" ? "team_id" : "id", "teamId")
  const result = parseRecord(tool.result)

  if (family === "team" && action === "resume") {
    const status = normalizedField(result, "status")
    const teamStatus = normalizedField(result, "teamStatus", "team_status")
    const disposition = normalizedField(result, "disposition")
    const resumeSummary = teamResumeSummary(result)
    const common = { targetId, resumeSummary }
    if (disposition === "no_op" || status === "already_complete") {
      return makeItem(tool, family, action, "completed", {
        ...common,
        outcome: "no_action_needed",
      })
    }
    if (disposition === "refused" || status === "refused" || status === "paused") {
      return makeItem(tool, family, action, "refused", common)
    }
    if (
      disposition === "partial" ||
      status === "partially_resumed" ||
      (resumeSummary?.failedCount ?? 0) > 0 ||
      (resumeSummary?.failures.length ?? 0) > 0
    ) {
      return makeItem(tool, family, action, "partial", common)
    }
    const baseState = baseToolState(tool)
    if (baseState) return makeItem(tool, family, action, baseState, common)
    if (resultWasRefused(result)) return makeItem(tool, family, action, "refused", common)
    if (
      (disposition === "resumed" && teamStatus === "active") ||
      (status === "resumed" && (teamStatus === undefined || teamStatus === "active"))
    ) {
      return makeItem(tool, family, action, "completed", common)
    }
    return makeItem(tool, family, action, "accepted", common)
  }

  const baseState = baseToolState(tool)
  if (baseState) return makeItem(tool, family, action, baseState, { targetId })
  if (resultWasRefused(result)) return makeItem(tool, family, action, "refused", { targetId })

  if (family === "team") {
    const status = normalizedField(result, "status")
    if (action === "pause" && status === "paused") {
      return makeItem(tool, family, action, "completed", { targetId })
    }
  }

  // Cron currently returns free-form text and MessageContent has no live Cron
  // projection. Keep it accepted instead of inferring state from prose.
  return makeItem(tool, family, action, "accepted", { targetId })
}

function parseWorkflowActivity(
  tool: ToolCall,
  args: JsonRecord,
): RuntimeControlActivityItem | null {
  if (normalizedField(args, "action") !== "control") return null
  const command = normalizedField(args, "command")
  if (command !== "pause" && command !== "resume" && command !== "cancel") return null
  const targetId = stringField(args, "runId", "run_id")
  const baseState = baseToolState(tool)
  if (baseState) return makeItem(tool, "workflow", command, baseState, { targetId })
  const result = parseRecord(tool.result)
  if (resultWasRefused(result)) {
    return makeItem(tool, "workflow", command, "refused", { targetId })
  }

  const run = nestedRecord(result?.run)
  const runState = normalizedField(run, "state", "status")
  const confirmed =
    (command === "pause" && runState === "paused") ||
    (command === "resume" && runState === "running") ||
    (command === "cancel" && (runState === "cancelled" || runState === "canceled"))
  if (confirmed) return makeItem(tool, "workflow", command, "completed", { targetId })

  // A successful tool invocation without the expected durable run state is
  // only an accepted request; its message field is not a state source.
  return makeItem(tool, "workflow", command, "accepted", { targetId })
}

/**
 * Parse one tool call into a runtime-control activity. This is deliberately a
 * pure projection: it never mutates lifecycle state and never derives owner
 * authority from IDs used only for display.
 */
export function parseRuntimeControlActivity(
  tool: ToolCall,
  context: RuntimeControlParseContext = {},
): RuntimeControlActivityItem | null {
  const args = parseRecord(tool.arguments)
  if (!args) return null

  switch (tool.name) {
    case "subagent":
      return parseSubagentActivity(tool, args, context)
    case "job_status":
      return parseJobCancelActivity(tool, args)
    case "process":
      return parseNativeProcessActivity(tool, args)
    case "runtime_cancel":
      return parseRuntimeCancelActivity(tool, args, context)
    case "team":
      return parseAcceptedControl(tool, args, "team")
    case "workflow":
      return parseWorkflowActivity(tool, args)
    case "manage_cron":
      return parseAcceptedControl(tool, args, "cron")
    default:
      return null
  }
}

export function getRuntimeControlActivityGroupKey(
  activity: Pick<RuntimeControlActivityItem, "family" | "action" | "allTargets" | "outcome">,
): string {
  const outcomeBucket = activity.outcome === "no_action_needed" ? "no_action_needed" : "action"
  return `${activity.family}:${activity.action}:${activity.allTargets ? "all" : "targets"}:${outcomeBucket}`
}
