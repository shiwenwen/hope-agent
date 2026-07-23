import { DEFAULT_AGENT_ID } from "@/types/tools"
import type { SubagentRun, ToolCall } from "@/types/chat"

/** One sub-agent run derived from a `subagent` spawn-like tool_call block. A block
 *  can yield 0 items (non-spawn action / parse failure), 1 (spawn / send / resume / spawn_and_wait),
 *  or N (batch_spawn). Before the tool result lands the item is `pending` — it
 *  carries only what the spawn arguments declared and must be aligned to a real
 *  run (see {@link matchPendingRun}) to become clickable. */
export type SubagentChipItem =
  | {
      kind: "resolved"
      /** Stable identity for React keys — `${callId}` / `${callId}:${idx}` — the
       *  SAME value a still-pending item for this slot carries, so a chip does
       *  not remount when its result lands and it flips pending → resolved. */
      key: string
      runId: string
      agentId: string
      task: string
      label?: string
    }
  | {
      kind: "pending"
      /** Stable identity for React keys: `${callId}` or `${callId}:${idx}` (batch). */
      key: string
      agentId: string
      task: string
      label?: string
      startedAtMs?: number
    }

/** What a chip click hands to the panel. `runId === null` means the spawn is
 *  still in-flight / unaligned — open the panel but select nothing yet. */
export interface SubagentOpenTarget {
  runId: string | null
  childSessionId?: string
  agentId: string
  task: string
  label?: string
}

const SPAWN_ACTIONS = new Set(["spawn", "send", "resume", "spawn_and_wait", "batch_spawn"])

interface SubagentArgs {
  action?: string
  agent_id?: string
  task?: string
  message?: string
  label?: string
  tasks?: Array<{ agent_id?: string; task?: string; label?: string }>
}

/** Replaces the legacy `extractSubagentRuns`. With a result present the output
 *  is identical in shape to before (resolved runs; batch keeps only "spawned"
 *  entries; a missing run_id yields [] so the caller falls back to a plain tool
 *  block showing the error). Without a result it emits pending placeholders from
 *  the arguments alone so a chip appears the instant the spawn is issued. */
export function extractSubagentChipItems(tool: ToolCall): SubagentChipItem[] {
  if (tool.name !== "subagent") return []
  let args: SubagentArgs
  try {
    args = JSON.parse(tool.arguments)
  } catch {
    return []
  }
  if (!args.action || !SPAWN_ACTIONS.has(args.action)) return []

  if (tool.result) {
    let result: unknown
    try {
      result = JSON.parse(tool.result)
    } catch {
      return []
    }
    if (!result || typeof result !== "object") return []

    if (
      args.action === "spawn" ||
      args.action === "send" ||
      args.action === "resume" ||
      args.action === "spawn_and_wait"
    ) {
      const single = result as { run_id?: unknown; child_agent_id?: unknown }
      const runId = single.run_id
      if (typeof runId !== "string" || !runId) return []
      const resultAgentId =
        typeof single.child_agent_id === "string" && single.child_agent_id
          ? single.child_agent_id
          : undefined
      return [
        {
          kind: "resolved",
          key: tool.callId,
          runId,
          agentId: resultAgentId || args.agent_id || DEFAULT_AGENT_ID,
          task: args.task || args.message || "",
          label: args.label,
        },
      ]
    }

    // batch_spawn — one resolved item per successfully-spawned entry. Key on the
    // runs-array index so it matches the pending item's `${callId}:${idx}`.
    const runs = (result as { runs?: unknown }).runs
    if (!Array.isArray(runs)) return []
    const taskDefs = Array.isArray(args.tasks) ? args.tasks : []
    const out: SubagentChipItem[] = []
    for (let idx = 0; idx < runs.length; idx++) {
      const r = runs[idx]
      if (!r || typeof r !== "object") continue
      const obj = r as { status?: unknown; run_id?: unknown }
      if (obj.status !== "spawned") continue
      if (typeof obj.run_id !== "string" || !obj.run_id) continue
      const def = taskDefs[idx] || {}
      out.push({
        kind: "resolved",
        key: `${tool.callId}:${idx}`,
        runId: obj.run_id,
        agentId: def.agent_id || DEFAULT_AGENT_ID,
        task: def.task || "",
        label: def.label,
      })
    }
    return out
  }

  // `send` is state-dependent: it may steer the current attempt without
  // creating anything, or resume a terminal thread into a fresh attempt. Do
  // not guess before its result lands; once resolved, link to the authoritative
  // run_id returned by the backend.
  if (args.action === "send") return []

  // No result yet → pending placeholder(s) from arguments only.
  const startedAtMs = tool.startedAtMs
  if (args.action === "spawn" || args.action === "resume" || args.action === "spawn_and_wait") {
    return [
      {
        kind: "pending",
        key: tool.callId,
        agentId: args.agent_id || DEFAULT_AGENT_ID,
        task: args.task || "",
        label: args.label,
        startedAtMs,
      },
    ]
  }
  const taskDefs = Array.isArray(args.tasks) ? args.tasks : []
  return taskDefs.map((def, idx) => ({
    kind: "pending" as const,
    key: `${tool.callId}:${idx}`,
    agentId: def.agent_id || DEFAULT_AGENT_ID,
    task: def.task || "",
    label: def.label,
    startedAtMs,
  }))
}

/** Index a DESC-ordered run list by child session while retaining the newest
 *  continuation. Resume deliberately creates multiple immutable run rows for
 *  one child session; `new Map(runs.map(...))` would let the oldest row win
 *  because later duplicate keys overwrite earlier ones. */
export function indexLatestRunByChildSession(
  runs: SubagentRun[],
): ReadonlyMap<string, SubagentRun> {
  const indexed = new Map<string, SubagentRun>()
  for (const run of runs) {
    if (run.childSessionId && !indexed.has(run.childSessionId)) {
      indexed.set(run.childSessionId, run)
    }
  }
  return indexed
}

/**
 * Reduce markdown to one scannable plain-text line for list previews.
 *
 * Removes delimiters only, never content. Emphasis is stripped as balanced
 * PAIRS — a blanket `[*_~]` sweep would turn `model_campaign` into
 * `modelcampaign` and `__init__` into `init`. Underscores are left alone
 * entirely for the same reason: `_italic_` is far rarer in a coding agent's
 * output than snake_case identifiers. Ordered-list numbers are kept because
 * they carry meaning (and `2024. …` would be mangled).
 */
export function markdownPreview(text: string): string {
  return text
    .replace(/```[\s\S]*?```/g, " ") // fenced code — the case that forced this
    .replace(/~~~[\s\S]*?~~~/g, " ")
    .replace(/!\[[^\]]*\]\([^)]*\)/g, " ") // images carry nothing readable
    .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1") // links → their text, drop the URL
    .replace(/`([^`]*)`/g, "$1") // inline-code delimiters only
    .replace(/^\s{0,3}#{1,6}\s+/gm, "") // leading heading markers
    .replace(/^\s{0,3}[-*+]\s+/gm, "") // leading bullets
    .replace(/^\s{0,3}>\s?/gm, "") // leading block quotes
    .replace(/\*\*\*(?=\S)([\s\S]*?\S)\*\*\*/g, "$1") // ***bold italic***
    .replace(/\*\*(?=\S)([\s\S]*?\S)\*\*/g, "$1") // **bold**
    .replace(/\*(?=\S)([^*\n]*?\S)\*/g, "$1") // *italic*
    .replace(/~~(?=\S)([\s\S]*?\S)~~/g, "$1") // ~~strike~~
    .replace(/\s+/g, " ")
    .trim()
}

/** Align a pending chip to a real run from the session snapshot. `runs` is
 *  started_at DESC; we scan oldest-first so sequential pending chips for an
 *  identical (agent, task) pair claim distinct runs in spawn order. A run is a
 *  candidate when agent + task match, it isn't already claimed by an earlier
 *  chip in this row, and it started within a window around the tool call —
 *  the run is created during spawn execution, so it lands at ≈ the tool's
 *  start time. Bounding both sides (15s early for clock skew, 60s late) stops
 *  a long-pending chip from binding an unrelated much-newer run of the same
 *  shape. */
export function matchPendingRun(
  runs: SubagentRun[],
  pending: { agentId: string; task: string },
  startedAtMs: number | undefined,
  claimed: ReadonlySet<string>,
): SubagentRun | null {
  for (let idx = runs.length - 1; idx >= 0; idx--) {
    const run = runs[idx]
    if (run.childAgentId !== pending.agentId) continue
    if (run.task !== pending.task) continue
    if (claimed.has(run.runId)) continue
    if (startedAtMs !== undefined) {
      const started = Date.parse(run.startedAt)
      if (
        !Number.isNaN(started) &&
        (started < startedAtMs - 15_000 || started > startedAtMs + 60_000)
      )
        continue
    }
    return run
  }
  return null
}
