#!/usr/bin/env node
import childProcess from "node:child_process"
import fs from "node:fs"
import net from "node:net"
import os from "node:os"
import path from "node:path"

const defaultPlansDir = path.join(
  os.homedir(),
  "Library/Mobile Documents/com~apple~CloudDocs/HopeAI/Hope Agent/Plans/hope-agent-control-plane-plans-2026-07-05/11-agent-control-plane-v3-claude-parity",
)

const args = parseArgs(process.argv.slice(2))

if (args.help) {
  printHelp()
  process.exit(0)
}

const startedAt = new Date().toISOString()
const plansDir = path.resolve(args.plansDir ?? process.env.HOPE_V3_PLANS_DIR ?? defaultPlansDir)
const artifactPath = path.resolve(args.artifact ?? path.join(plansDir, "evidence/wall-clock-soak-2026-07-08.md"))
const runId = args.runId ?? `wall-clock-soak-${new Date().toISOString().replace(/[-:]/g, "").replace(/\..+/, "")}`
const dataDir = path.resolve(args.dataDir ?? path.join(os.tmpdir(), `hope-agent-v3-${runId}`))
const stateFile = path.resolve(args.stateFile ?? path.join(dataDir, "wall-clock-soak-state.json"))
const logDir = path.join(dataDir, "wall-clock-soak-logs")
const launchApp = !args.baseUrl && !args.noLaunch
const vitePort = launchApp ? await choosePort(args.vitePort) : args.vitePort
const serverPort = args.baseUrl ? null : launchApp ? await choosePort(args.serverPort) : args.serverPort
const identifier = args.identifier ?? `ai.hopeagent.desktop.v3soak.${process.pid}`
const baseUrl = (args.baseUrl ?? `http://127.0.0.1:${serverPort}`).replace(/\/$/, "")
const events = []

fs.mkdirSync(logDir, { recursive: true })

let launcher = null
try {
  record("soak_start", {
    runId,
    dataDir,
    stateFile,
    artifactPath,
    baseUrl,
    launchApp,
    requestedSoakSecs: args.soakSecs,
    loopFallbackSecs: args.loopFallbackSecs,
    identifier,
  })

  if (launchApp) {
    launcher = startLauncher("initial")
  }
  await waitForHealth(baseUrl, args.waitHealthSecs)
  record("health_ready", await api("GET", "/api/health", null, { public: true }))

  const session = args.sessionId
    ? { id: args.sessionId, reused: true }
    : await api("POST", "/api/sessions", { incognito: false })
  record(args.sessionId ? "session_reused" : "session_created", { sessionId: session.id })

  const goal = await api("POST", `/api/sessions/${encodeURIComponent(session.id)}/goal`, {
    objective:
      "V3 wall-clock soak: prove Goal, Loop, and Workflow control-plane state stays coherent across real elapsed time without mutating production data.",
    completionCriteria: [
      "[required] Real wall-clock start/end timestamps are recorded; no synthetic timestamp is used.",
      "[required] Goal remains durable and actionable through the soak window.",
      "[required] Loop records at least one natural scheduled trigger, not only manual run-now.",
      "[required] Workflow records a visible phase/checkpoint/result state.",
      "[required] Final durable store state can be read back through owner APIs.",
    ].join("\n"),
    domain: "general",
    budgetTimeLimitSecs: args.soakSecs + 900,
    budgetTurnLimit: 12,
  })
  const goalId = goal.goal?.id ?? goal.id
  record("goal_created", summarizeGoal(goal))

  const guardTasks = await api("POST", `/api/sessions/${encodeURIComponent(session.id)}/tasks`, {
    content:
      "V3 wall-clock soak continuation guard: keep this Goal open until a natural non-skipped Loop continuation is observed.",
    activeForm: "Waiting for natural Loop continuation",
  })
  const guardTask = Array.isArray(guardTasks) ? guardTasks.at(-1) : null
  record("goal_continuation_guard_task_created", guardTask)

  const workflow = await api("POST", `/api/sessions/${encodeURIComponent(session.id)}/workflow-runs`, {
    kind: "general.workflow",
    executionMode: "guarded",
    scriptSource: wallClockWorkflowScript(),
    origin: `strict-proof:wall-clock-soak:${runId}`,
    goalId,
    runImmediately: true,
    budget: {
      maxOps: 24,
      maxRuntimeSecs: 300,
    },
  })
  const workflowRunId = workflow.id
  record("workflow_created", summarizeWorkflow({ run: workflow }))

  const workflowReady = await waitForWorkflowTerminalOrProgress(workflowRunId, args.workflowTimeoutSecs)
  record("workflow_progress_or_terminal", summarizeWorkflow(workflowReady))

  const loop = await api("POST", `/api/sessions/${encodeURIComponent(session.id)}/loops`, {
    triggerKind: "dynamic",
    triggerSpec: {
      fallbackSecs: args.loopFallbackSecs,
      fallbackUsed: false,
    },
    executionStrategy: "continue",
    prompt:
      "V3 wall-clock soak natural Loop sample: when triggered, inspect current Goal/Workflow state, record whether more work is needed, then choose a next check time or stop. Do not modify files or external systems.",
    goalId,
    maxRuns: 2,
    maxRuntimeSecs: args.soakSecs + 900,
    tokenBudget: 20000,
    maxNoProgressRuns: 2,
    maxFailures: 2,
    backoffSecs: Math.max(60, args.loopFallbackSecs),
  })
  const loopId = loop.id
  record("loop_created_waiting_for_natural_trigger", summarizeLoopSnapshot({ schedule: loop, runs: [] }))

  const state = {
    baseUrl,
    dataDir,
    goalId,
    identifier,
    loopId,
    runId,
    sessionId: session.id,
    startedAt,
    workflowRunId,
  }
  writeJson(stateFile, state)

  const waitStarted = Date.now()
  const naturalLoopSnapshot = await waitForNaturalLoopRun(loopId, args.waitLoopSecs)
  record("natural_loop_run_observed", summarizeLoopSnapshot(naturalLoopSnapshot))
  if (guardTask?.id && getPositiveContinuationRuns(naturalLoopSnapshot).length > 0) {
    const updatedTasks = await api("PATCH", `/api/tasks/${encodeURIComponent(guardTask.id)}/status`, {
      status: "completed",
    })
    record("goal_continuation_guard_task_completed", {
      taskId: guardTask.id,
      taskCount: Array.isArray(updatedTasks) ? updatedTasks.length : null,
    })
  }

  const remainingMs = args.soakSecs * 1000 - (Date.now() - waitStarted)
  if (remainingMs > 0) {
    record("soak_remaining_wait_started", {
      remainingSecs: Math.ceil(remainingMs / 1000),
      reason: "natural loop fired before requested wall-clock duration elapsed",
    })
    await sleep(remainingMs)
  }

  const finalGoalReadBack = await readFinalGoal(goalId)
  const finalGoal = finalGoalReadBack.snapshot
  const finalWorkflow = await api("GET", `/api/workflow-runs/${encodeURIComponent(workflowRunId)}`)
  const finalLoop = await api("GET", `/api/loops/${encodeURIComponent(loopId)}`)
  const goalWatchdog = await api("GET", `/api/sessions/${encodeURIComponent(session.id)}/goal/watchdog?staleSecs=1`)
  const loopWatchdog = await api("GET", `/api/sessions/${encodeURIComponent(session.id)}/loops/watchdog?graceSecs=1`)
  const workflowWatchdog = await api(
    "GET",
    `/api/sessions/${encodeURIComponent(session.id)}/workflow-runs/watchdog?staleSecs=1`,
  )

  record("final_goal_read_back", {
    source: finalGoalReadBack.source,
    evaluateError: finalGoalReadBack.evaluateError,
    goal: summarizeGoal(finalGoal),
  })
  record("final_workflow", summarizeWorkflow(finalWorkflow))
  record("final_loop", summarizeLoopSnapshot(finalLoop))
  record("watchdogs", {
    goal: goalWatchdog,
    loop: loopWatchdog,
    workflow: workflowWatchdog,
  })

  const finishedAt = new Date().toISOString()
  const result = {
    status: "completed",
    runId,
    startedAt,
    finishedAt,
    elapsedSecs: Math.floor((Date.parse(finishedAt) - Date.parse(startedAt)) / 1000),
    baseUrl,
    dataDir,
    stateFile,
    ids: {
      sessionId: session.id,
      goalId,
      guardTaskId: guardTask?.id ?? null,
      loopId,
      loopRunIds: Array.isArray(finalLoop?.runs) ? finalLoop.runs.map((run) => run.id) : [],
      workflowRunId,
    },
    coverageObservations: buildCoverageObservations({
      finalGoal,
      finalLoop,
      finalWorkflow,
      finishedAt,
      naturalLoopSnapshot,
      startedAt,
    }),
    events,
  }
  appendArtifact(result)
  printResult(result)
} catch (error) {
  record("soak_failed", { message: error.message })
  const failed = {
    status: "failed",
    runId,
    startedAt,
    finishedAt: new Date().toISOString(),
    baseUrl,
    dataDir,
    stateFile,
    error: error.message,
    events,
  }
  appendArtifact(failed)
  process.stderr.write(`${error.stack ?? error.message}\n`)
  process.exitCode = 1
} finally {
  if (launcher) killLauncher(launcher, "cleanup")
}

function wallClockWorkflowScript() {
  return `export default async function main(workflow) {
  const task = await workflow.task.create({ title: "V3 wall-clock soak workflow evidence" });
  await workflow.phase(
    {
      name: "Control-plane checkpoint",
      label: "control-plane-checkpoint",
      expected: "Record a visible phase/checkpoint/result without touching production data."
    },
    async (phase) => {
      await workflow.progress({
        phaseKey: phase.phaseKey,
        message: "Workflow phase is alive during the wall-clock soak sample.",
        counters: { checkpoints: 1 },
        importance: "medium"
      });
      await workflow.checkpoint({
        title: "Wall-clock soak workflow checkpoint",
        summary: "Workflow produced a visible checkpoint for the V3 real soak proof.",
        phaseKey: phase.phaseKey,
        importance: "high",
        inject: "never",
        findings: ["No external systems touched.", "Checkpoint is durable in workflow trace."]
      });
    }
  );
  const validation = await workflow.validate({
    reason: "V3 wall-clock soak local validation; harmless stdout-only command.",
    label: "local-validation",
    commands: [{ command: "printf v3-wall-clock-soak-validation-ok", timeout: 10 }]
  });
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({
    summary: "V3 wall-clock soak workflow completed with checkpoint and local validation.",
    validation
  });
}
`
}

function buildCoverageObservations({ finalGoal, finalLoop, finalWorkflow, finishedAt, naturalLoopSnapshot, startedAt }) {
  const elapsedSecs = Math.floor((Date.parse(finishedAt) - Date.parse(startedAt)) / 1000)
  const loopRuns = Array.isArray(finalLoop?.runs) ? finalLoop.runs : []
  const naturalRuns = loopRuns.filter((run) => run.triggerReason !== "manual_run_now")
  const positiveContinuationRuns = getPositiveContinuationRuns({ runs: naturalRuns })
  const workflowEvents = Array.isArray(finalWorkflow?.events) ? finalWorkflow.events : []
  const workflowHasVisibleState =
    workflowEvents.some((event) => ["phase_started", "phase_completed", "checkpoint", "progress", "trace"].includes(event.eventType ?? event.event_type)) ||
    ["completed", "blocked", "failed"].includes(finalWorkflow?.run?.state ?? finalWorkflow?.state)
  const finalGoalState = finalGoal?.goal?.state ?? finalGoal?.state ?? null
  return {
    wall_clock: {
      observed: elapsedSecs >= args.soakSecs,
      elapsedSecs,
      requestedSoakSecs: args.soakSecs,
      note: "Uses actual process time between runner start and finish.",
    },
    goal_continuation: {
      observed: positiveContinuationRuns.length > 0,
      durableStateObserved: Boolean(finalGoal?.goal ?? finalGoal),
      state: finalGoalState,
      positiveContinuationRunCount: positiveContinuationRuns.length,
      note:
        positiveContinuationRuns.length > 0
          ? "Observed at least one natural Loop run that completed/succeeded instead of being skipped or failed."
          : "Durable Goal state was readable, but no successful natural model continuation run was observed. Do not count this as strict Goal continuation.",
    },
    loop_reschedule: {
      observed: naturalRuns.length > 0 || Boolean(naturalLoopSnapshot),
      runCount: loopRuns.length,
      naturalRunCount: naturalRuns.length,
      nextRunAt: finalLoop?.schedule?.nextRunAt ?? finalLoop?.nextRunAt ?? null,
      note: "Runner never calls run-now; observed Loop runs are natural Cron/Loop triggers.",
    },
    workflow_recovery: {
      observed: workflowHasVisibleState,
      state: finalWorkflow?.run?.state ?? finalWorkflow?.state ?? null,
      eventCount: workflowEvents.length,
      note: "Workflow produced a phase/checkpoint/result state; recovery is not forced in this soak sample.",
    },
  }
}

function appendArtifact(result) {
  fs.mkdirSync(path.dirname(artifactPath), { recursive: true })
  const coverage = result.coverageObservations ?? {}
  const lines = [
    "",
    `## Wall-clock Soak Runner - ${result.finishedAt}`,
    "",
    `- Status: \`${result.status}\``,
    `- Run id: \`${result.runId}\``,
    `- Base URL: \`${result.baseUrl}\``,
    `- Data dir: \`${result.dataDir}\``,
    `- State file: \`${result.stateFile}\``,
    `- Start: \`${result.startedAt}\``,
    `- Finish: \`${result.finishedAt}\``,
    `- Elapsed seconds: \`${result.elapsedSecs ?? "n/a"}\``,
    `- Session id: \`${result.ids?.sessionId ?? ""}\``,
    `- Goal id: \`${result.ids?.goalId ?? ""}\``,
    `- Goal continuation guard task id: \`${result.ids?.guardTaskId ?? ""}\``,
    `- Loop id: \`${result.ids?.loopId ?? ""}\``,
    `- Loop run ids: \`${(result.ids?.loopRunIds ?? []).join(", ") || ""}\``,
    `- Workflow run id: \`${result.ids?.workflowRunId ?? ""}\``,
    "",
    result.status === "completed"
      ? "This runner records a real wall-clock owner-API soak. It does not check coverage boxes or mark the strict proof as passed; a reviewer must decide whether the evidence is strong enough, especially for model-level Goal continuation."
      : `Runner failed before completing the soak sample: ${result.error}`,
    "",
    "### Coverage Observations",
    "",
    `- wall_clock: \`${coverage.wall_clock?.observed ?? false}\` (${coverage.wall_clock?.elapsedSecs ?? "n/a"}s elapsed; requested ${coverage.wall_clock?.requestedSoakSecs ?? "n/a"}s)`,
    `- goal_continuation: \`${coverage.goal_continuation?.observed ?? false}\` (${coverage.goal_continuation?.state ?? "unknown"})`,
    `- loop_reschedule: \`${coverage.loop_reschedule?.observed ?? false}\` (${coverage.loop_reschedule?.naturalRunCount ?? 0} natural run(s); nextRunAt ${coverage.loop_reschedule?.nextRunAt ?? "n/a"})`,
    `- workflow_recovery: \`${coverage.workflow_recovery?.observed ?? false}\` (${coverage.workflow_recovery?.state ?? "unknown"}; ${coverage.workflow_recovery?.eventCount ?? 0} event(s))`,
    "",
    "### Raw Runner Result",
    "",
    "```json",
    JSON.stringify(result, null, 2),
    "```",
    "",
  ]
  fs.appendFileSync(artifactPath, lines.join("\n"))
}

function startLauncher(label) {
  const stdoutPath = path.join(logDir, `${label}-launcher.stdout.log`)
  const stderrPath = path.join(logDir, `${label}-launcher.stderr.log`)
  const stdout = fs.openSync(stdoutPath, "a")
  const stderr = fs.openSync(stderrPath, "a")
  const child = childProcess.spawn(
    process.execPath,
    [
      "scripts/v3-tauri-smoke-launch.mjs",
      "--run",
      "--force",
      "--data-dir",
      dataDir,
      "--vite-port",
      String(vitePort),
      "--server-port",
      String(serverPort),
      "--identifier",
      identifier,
    ],
    {
      cwd: process.cwd(),
      detached: true,
      stdio: ["ignore", stdout, stderr],
    },
  )
  child.unref()
  record("launcher_started", { label, pid: child.pid, stdoutPath, stderrPath })
  return { label, pid: child.pid, stderrPath, stdoutPath }
}

function killLauncher(child, reason) {
  if (!child?.pid) return
  try {
    process.kill(-child.pid, "SIGTERM")
    record("launcher_killed", { reason, pid: child.pid, signal: "SIGTERM" })
  } catch (error) {
    record("launcher_kill_warning", { reason, pid: child.pid, message: error.message })
  }
}

async function waitForWorkflowTerminalOrProgress(runId, timeoutSecs) {
  const deadline = Date.now() + timeoutSecs * 1000
  let latest = null
  while (Date.now() < deadline) {
    latest = await api("GET", `/api/workflow-runs/${encodeURIComponent(runId)}`)
    const state = latest?.run?.state ?? latest?.state
    const events = Array.isArray(latest?.events) ? latest.events : []
    if (["completed", "blocked", "failed", "awaiting_approval", "awaiting_user"].includes(state) || events.length >= 2) {
      return latest
    }
    await sleep(750)
  }
  return latest
}

async function waitForNaturalLoopRun(loopId, timeoutSecs) {
  const deadline = Date.now() + timeoutSecs * 1000
  let latest = null
  while (Date.now() < deadline) {
    latest = await api("GET", `/api/loops/${encodeURIComponent(loopId)}`)
    const runs = Array.isArray(latest?.runs) ? latest.runs : []
    const naturalRuns = runs.filter((run) => run.triggerReason !== "manual_run_now")
    if (naturalRuns.length > 0) return latest
    await sleep(1000)
  }
  throw new Error(`Loop ${loopId} did not record a natural run within ${timeoutSecs}s`)
}

function getPositiveContinuationRuns(snapshot) {
  const runs = Array.isArray(snapshot?.runs) ? snapshot.runs : []
  return runs.filter((run) => {
    if (run.triggerReason === "manual_run_now" || run.triggerReason === "goal_completed") return false
    const succeeded = run.state === "succeeded" || run.state === "completed"
    const usage = run.usage ?? {}
    const hasModelEvidence =
      (usage.providerEvents ?? 0) > 0 ||
      (usage.assistantMessages ?? 0) > 0 ||
      (usage.providerTotalTokens ?? 0) > 0 ||
      (usage.totalTokens ?? 0) > 0
    return succeeded && hasModelEvidence
  })
}

async function api(method, route, body = null, options = {}) {
  const response = await apiRaw(method, route, body, options)
  if (!response.ok) {
    throw new Error(`${method} ${route} -> ${response.status}: ${JSON.stringify(response.body)}`)
  }
  return response.body
}

async function readFinalGoal(goalId) {
  const evaluate = await apiRaw("POST", `/api/goals/${encodeURIComponent(goalId)}/evaluate`, {})
  if (evaluate.ok) {
    return { evaluateError: null, snapshot: evaluate.body, source: "evaluate" }
  }
  const readBack = await api("GET", `/api/goals/${encodeURIComponent(goalId)}`)
  return {
    evaluateError: `${evaluate.status}: ${JSON.stringify(evaluate.body)}`,
    snapshot: readBack,
    source: "read_back_after_evaluate_rejected",
  }
}

async function apiRaw(method, route, body = null, options = {}) {
  const headers = body === null ? {} : { "content-type": "application/json" }
  if (args.token && !options.public) headers.authorization = `Bearer ${args.token}`
  const response = await fetch(`${baseUrl}${route}`, {
    method,
    headers,
    body: body === null ? undefined : JSON.stringify(body),
  })
  const text = await response.text()
  let parsed = null
  if (text.trim()) {
    try {
      parsed = JSON.parse(text)
    } catch {
      parsed = text
    }
  }
  return {
    body: parsed,
    ok: response.ok,
    status: response.status,
  }
}

async function waitForHealth(baseUrl, timeoutSecs) {
  const deadline = Date.now() + timeoutSecs * 1000
  let lastError = null
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`${baseUrl}/api/health`)
      if (response.ok) return
      lastError = new Error(`health returned ${response.status}`)
    } catch (error) {
      lastError = error
    }
    await sleep(750)
  }
  throw new Error(`Health check did not become ready within ${timeoutSecs}s: ${lastError?.message ?? "unknown error"}`)
}

function summarizeGoal(value) {
  const goal = value?.goal ?? value
  if (!goal) return null
  return {
    id: goal.id,
    state: goal.state,
    revision: goal.revision,
    closureDecision: goal.closureDecision ?? goal.closure_decision ?? null,
    evidence: value?.evidence?.length ?? null,
    tasks: value?.tasks?.length ?? null,
  }
}

function summarizeWorkflow(value) {
  const run = value?.run ?? value
  if (!run) return null
  return {
    id: run.id,
    state: run.state,
    kind: run.kind,
    origin: run.origin,
    events: value?.events?.length ?? null,
    ops: value?.ops?.length ?? null,
  }
}

function summarizeLoopSnapshot(snapshot) {
  const schedule = snapshot?.schedule ?? snapshot
  const runs = Array.isArray(snapshot?.runs) ? snapshot.runs : []
  if (!schedule) return null
  return {
    id: schedule.id ?? null,
    state: schedule.state ?? null,
    triggerKind: schedule.triggerKind ?? schedule.trigger_kind ?? null,
    executionStrategy: schedule.executionStrategy ?? schedule.execution_strategy ?? null,
    cronStatus: schedule.cronStatus ?? schedule.cron_status ?? null,
    nextRunAt: schedule.nextRunAt ?? schedule.next_run_at ?? null,
    runCount: runs.length,
    runs: runs.map((run) => ({
      id: run.id,
      state: run.state,
      seq: run.seq,
      triggerReason: run.triggerReason,
      schedulingDecision: run.schedulingDecision,
      progressState: run.progressState,
      noProgressReason: run.noProgressReason,
      usage: run.usage ?? null,
      startedAt: run.startedAt,
      finishedAt: run.finishedAt,
    })),
  }
}

async function choosePort(preferred) {
  let port = preferred
  while (!(await isPortFree(port))) port += 1
  return port
}

function isPortFree(port) {
  return new Promise((resolve) => {
    const server = net.createServer()
    server.once("error", () => resolve(false))
    server.once("listening", () => {
      server.close(() => resolve(true))
    })
    server.listen(port, "127.0.0.1")
  })
}

function parseArgs(argv) {
  const parsed = {
    artifact: null,
    baseUrl: null,
    dataDir: null,
    help: false,
    identifier: null,
    json: false,
    loopFallbackSecs: 120,
    noLaunch: false,
    plansDir: null,
    runId: null,
    serverPort: 18440,
    sessionId: null,
    soakSecs: 1800,
    stateFile: null,
    token: process.env.HOPE_API_TOKEN ?? null,
    vitePort: 1440,
    waitHealthSecs: 90,
    waitLoopSecs: 600,
    workflowTimeoutSecs: 120,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === "--artifact") parsed.artifact = argv[++i]
    else if (arg === "--base-url") parsed.baseUrl = argv[++i]
    else if (arg === "--data-dir") parsed.dataDir = argv[++i]
    else if (arg === "--help" || arg === "-h") parsed.help = true
    else if (arg === "--identifier") parsed.identifier = argv[++i]
    else if (arg === "--json") parsed.json = true
    else if (arg === "--loop-fallback-secs") parsed.loopFallbackSecs = parsePositiveInt(argv[++i], "--loop-fallback-secs")
    else if (arg === "--no-launch") parsed.noLaunch = true
    else if (arg === "--plans-dir") parsed.plansDir = argv[++i]
    else if (arg === "--run-id") parsed.runId = argv[++i]
    else if (arg === "--server-port") parsed.serverPort = parsePort(argv[++i], "--server-port")
    else if (arg === "--session-id") parsed.sessionId = argv[++i]
    else if (arg === "--soak-secs") parsed.soakSecs = parsePositiveInt(argv[++i], "--soak-secs")
    else if (arg === "--state-file") parsed.stateFile = argv[++i]
    else if (arg === "--token") parsed.token = argv[++i]
    else if (arg === "--vite-port") parsed.vitePort = parsePort(argv[++i], "--vite-port")
    else if (arg === "--wait-health-secs") parsed.waitHealthSecs = parsePositiveInt(argv[++i], "--wait-health-secs")
    else if (arg === "--wait-loop-secs") parsed.waitLoopSecs = parsePositiveInt(argv[++i], "--wait-loop-secs")
    else if (arg === "--workflow-timeout-secs") parsed.workflowTimeoutSecs = parsePositiveInt(argv[++i], "--workflow-timeout-secs")
    else throw new Error(`Unknown argument: ${arg}`)
  }
  return parsed
}

function printHelp() {
  process.stdout.write(`Usage:
  node scripts/v3-wall-clock-soak-runner.mjs [options]

Purpose:
  Run a real wall-clock owner-API soak for V3 strict proof. The runner launches
  an isolated Tauri desktop instance, creates a durable Goal, starts a Workflow
  that records a visible checkpoint/result, creates a Dynamic Loop without
  calling run-now, waits for a natural Loop trigger and real elapsed time, then
  reads back durable state and appends evidence.

Important:
  This script records evidence only. It never checks coverage boxes and never
  marks strict proof as passed. The default soak is 30 minutes.

Modes:
  Default mode launches an isolated Tauri instance with an empty test data dir.
  Pass --base-url to attach to an already running, configured app/server instead;
  this is the preferred route when strict proof needs a real provider.

Options:
  --soak-secs <seconds>          Minimum real elapsed wait. Defaults to 1800.
  --loop-fallback-secs <seconds> Dynamic Loop fallback interval. Defaults to 120.
  --wait-loop-secs <seconds>     Max wait for first natural Loop run. Defaults to 600.
  --workflow-timeout-secs <secs> Max wait for Workflow progress/terminal. Defaults to 120.
  --base-url <url>               Attach to an existing owner API instead of launching Tauri.
  --token <token>                Optional Bearer token; also reads HOPE_API_TOKEN.
  --session-id <id>              Reuse an existing session on --base-url.
  --no-launch                    Do not launch Tauri; use --server-port/default base URL.
  --data-dir <path>              Reusable isolated HA_DATA_DIR.
  --state-file <path>            State JSON. Defaults to <data-dir>/wall-clock-soak-state.json.
  --artifact <path>              Evidence artifact to append.
  --plans-dir <path>             Override Plans directory.
  --vite-port <port>             Preferred Vite port. Defaults to 1440.
  --server-port <port>           Preferred embedded server port. Defaults to 18440.
  --identifier <id>              Tauri app identifier.
  --run-id <id>                  Stable label for logs/data dir.
  --json                         Print machine-readable result.
  --help, -h                     Show this help.
`)
}

function parsePositiveInt(value, flag) {
  const parsed = Number.parseInt(value, 10)
  if (!Number.isInteger(parsed) || parsed <= 0) throw new Error(`${flag} must be a positive integer.`)
  return parsed
}

function parsePort(value, flag) {
  const port = Number.parseInt(value, 10)
  if (!Number.isInteger(port) || port < 1 || port > 65535) throw new Error(`${flag} must be a TCP port.`)
  return port
}

function record(name, value) {
  events.push({ at: new Date().toISOString(), name, value })
}

function writeJson(file, value) {
  fs.mkdirSync(path.dirname(file), { recursive: true })
  fs.writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`)
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

function printResult(result) {
  if (args.json) {
    process.stdout.write(`${JSON.stringify(result, null, 2)}\n`)
    return
  }
  process.stdout.write(
    [
      `V3 wall-clock soak runner ${result.status}.`,
      `Run id: ${result.runId}`,
      `Elapsed seconds: ${result.elapsedSecs}`,
      `Artifact: ${artifactPath}`,
      `Session: ${result.ids?.sessionId ?? ""}`,
      `Goal: ${result.ids?.goalId ?? ""}`,
      `Loop: ${result.ids?.loopId ?? ""}`,
      `Workflow: ${result.ids?.workflowRunId ?? ""}`,
      "",
    ].join("\n"),
  )
}
