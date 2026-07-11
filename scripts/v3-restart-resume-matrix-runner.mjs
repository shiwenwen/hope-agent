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
const artifactPath = path.resolve(args.artifact ?? path.join(plansDir, "evidence/restart-resume-matrix-2026-07-08.md"))
const runId = args.runId ?? `matrix-${new Date().toISOString().replace(/[-:]/g, "").replace(/\..+/, "")}`
const dataDir = path.resolve(args.dataDir ?? path.join(os.tmpdir(), `hope-agent-v3-${runId}`))
const stateFile = path.resolve(args.stateFile ?? path.join(dataDir, "probe-state.json"))
const logDir = path.join(dataDir, "matrix-runner-logs")
const vitePort = await choosePort(args.vitePort)
const serverPort = await choosePort(args.serverPort)
const identifier = args.identifier ?? `ai.hopeagent.desktop.v3matrix.${process.pid}`
const baseUrl = `http://127.0.0.1:${serverPort}`
const events = []

fs.mkdirSync(logDir, { recursive: true })

let launcher = null
try {
  record("matrix_start", {
    runId,
    scenario: args.scenario,
    dataDir,
    stateFile,
    baseUrl,
    vitePort,
    serverPort,
    identifier,
  })

  launcher = startLauncher("initial")
  await waitForHealth(baseUrl, args.waitHealthSecs)
  record("initial_health_ready", { baseUrl })

  let sample
  if (args.scenario === "long-workflow") {
    sample = await runLongWorkflowRestartSample()
  } else if (args.scenario === "dynamic-loop") {
    sample = await runDynamicLoopRestartSample()
  } else if (args.scenario === "incognito-guard") {
    sample = await runIncognitoGuardSample()
  } else {
    throw new Error(`Unsupported scenario: ${args.scenario}`)
  }

  appendArtifact({
    status: "completed",
    state: sample.state,
    sample,
  })

  const result = {
    status: "completed",
    runId,
    scenario: args.scenario,
    dataDir,
    stateFile,
    artifactPath,
    baseUrl,
    startedAt,
    finishedAt: new Date().toISOString(),
    events,
  }
  printResult(result)
} catch (error) {
  record("matrix_failed", { message: error.message })
  appendArtifact({ status: "failed", error: error.message, state: safeReadJson(stateFile) })
  process.stderr.write(`${error.stack ?? error.message}\n`)
  process.exitCode = 1
} finally {
  if (launcher) {
    killLauncher(launcher, "cleanup")
  }
}

async function runLongWorkflowRestartSample() {
  const setup = runProbe("setup")
  record("probe_setup_finished", summarizeProbe(setup))
  const setupState = readJson(stateFile)
  record("state_after_setup", summarizeState(setupState))

  await restartIsolatedInstance("kill_during_running_window")

  const verify = runProbe("verify")
  record("probe_verify_finished", summarizeProbe(verify))
  const verifyState = readJson(stateFile)
  record("state_after_verify", summarizeState(verifyState))

  return {
    kind: "long-workflow",
    setup: summarizeProbe(setup),
    verify: summarizeProbe(verify),
    state: verifyState,
  }
}

async function runDynamicLoopRestartSample() {
  const session = await api("POST", "/api/sessions", { incognito: false })
  record("dynamic_loop_session_created", { sessionId: session.id })
  const loop = await api("POST", `/api/sessions/${encodeURIComponent(session.id)}/loops`, {
    triggerKind: "dynamic",
    triggerSpec: {
      fallbackSecs: args.dynamicLoopFallbackSecs,
      fallbackUsed: false,
    },
    executionStrategy: "continue",
    prompt:
      "V3 restart/resume dynamic Loop sample: record a real run/history entry; do not modify files or external systems.",
    maxRuns: 2,
    maxRuntimeSecs: 3600,
    tokenBudget: 20000,
    maxNoProgressRuns: 2,
    maxFailures: 2,
    backoffSecs: 120,
  })
  record("dynamic_loop_created", summarizeLoopSnapshot({ schedule: loop, runs: [] }))
  await api("POST", `/api/loops/${encodeURIComponent(loop.id)}/run-now`, {})
  record("dynamic_loop_run_now_requested", { loopId: loop.id })

  const setupSnapshot = await waitForLoopRuns(loop.id, 1, args.waitStateSecs)
  record("dynamic_loop_setup_snapshot", summarizeLoopSnapshot(setupSnapshot))

  const state = {
    sessionId: session.id,
    loopId: loop.id,
    scenario: args.scenario,
    setupAt: new Date().toISOString(),
    lastVerifiedAt: null,
    setupSnapshot,
  }
  writeJson(stateFile, state)

  await restartIsolatedInstance("kill_after_dynamic_loop_run_history")

  const verifySnapshot = await api("GET", `/api/loops/${encodeURIComponent(loop.id)}`)
  record("dynamic_loop_verify_snapshot", summarizeLoopSnapshot(verifySnapshot))
  const verifyState = {
    ...state,
    lastVerifiedAt: new Date().toISOString(),
    verifySnapshot,
  }
  writeJson(stateFile, verifyState)

  return {
    kind: "dynamic-loop",
    setup: summarizeLoopSnapshot(setupSnapshot),
    verify: summarizeLoopSnapshot(verifySnapshot),
    state: verifyState,
  }
}

async function runIncognitoGuardSample() {
  const session = await api("POST", "/api/sessions", { incognito: true })
  record("incognito_session_created", {
    sessionId: session.id,
    incognito: session.incognito ?? null,
  })
  const attempts = []
  attempts.push(
    await apiRaw("POST", `/api/sessions/${encodeURIComponent(session.id)}/goal`, {
      objective: "V3 incognito guard sample: this durable Goal must be rejected.",
      completionCriteria: "[required] Durable Goal creation is rejected in incognito.",
      domain: "general",
    }),
  )
  attempts.at(-1).name = "create_goal"
  attempts.push(
    await apiRaw("POST", `/api/sessions/${encodeURIComponent(session.id)}/loops`, {
      triggerKind: "dynamic",
      triggerSpec: { fallbackSecs: 120, fallbackUsed: false },
      executionStrategy: "continue",
      prompt: "V3 incognito guard sample: this durable Loop must be rejected.",
    }),
  )
  attempts.at(-1).name = "create_loop"
  attempts.push(
    await apiRaw("POST", `/api/sessions/${encodeURIComponent(session.id)}/workflow-runs`, {
      kind: "general.workflow",
      executionMode: "guarded",
      scriptSource:
        `export default async function main(workflow) {
  const task = await workflow.task.create({ title: "V3 incognito guard workflow should not persist" });
  await workflow.trace({ label: "incognito-guard-budget", payload: { maxOps: 4, maxRuntimeSecs: 60 } });
  const validation = await workflow.validate({
    reason: "V3 incognito guard validation should never run",
    commands: [{ command: "true", timeout: 10 }]
  });
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({ summary: "should not persist", validation });
}
`,
      origin: "strict-proof:incognito-guard",
      runImmediately: false,
      budget: { maxOps: 4, maxRuntimeSecs: 60 },
    }),
  )
  attempts.at(-1).name = "create_workflow"
  for (const attempt of attempts) {
    record("incognito_attempt", {
      name: attempt.name,
      ok: attempt.ok,
      status: attempt.status,
      body: attempt.body,
    })
  }
  const state = {
    sessionId: session.id,
    scenario: args.scenario,
    setupAt: new Date().toISOString(),
    attempts,
  }
  writeJson(stateFile, state)
  return {
    kind: "incognito-guard",
    setup: { sessionId: session.id, attempts },
    verify: null,
    state,
  }
}

async function restartIsolatedInstance(reason) {
  await sleep(args.killDelaySecs * 1000)
  killLauncher(launcher, reason)
  launcher = null
  await sleep(args.restartDelaySecs * 1000)
  launcher = startLauncher("restart")
  await waitForHealth(baseUrl, args.waitHealthSecs)
  record("restart_health_ready", { baseUrl })
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
  return { pid: child.pid, label, stdoutPath, stderrPath }
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

function runProbe(phase) {
  const commandArgs = [
    "scripts/v3-restart-resume-probe.mjs",
    "--phase",
    phase,
    "--scenario",
    args.scenario,
    "--base-url",
    baseUrl,
    "--state-file",
    stateFile,
    "--artifact",
    artifactPath,
    "--long-sleep-secs",
    String(args.longSleepSecs),
    "--wait-state-secs",
    String(args.waitStateSecs),
    "--json",
  ]
  if (phase === "verify" && args.exerciseControls) commandArgs.push("--exercise-controls")
  const output = childProcess.execFileSync(process.execPath, commandArgs, {
    cwd: process.cwd(),
    encoding: "utf8",
    maxBuffer: 1024 * 1024 * 20,
  })
  return JSON.parse(output)
}

async function api(method, route, body = null) {
  const response = await apiRaw(method, route, body)
  if (!response.ok) {
    throw new Error(`${method} ${route} -> ${response.status}: ${JSON.stringify(response.body)}`)
  }
  return response.body
}

async function apiRaw(method, route, body = null) {
  const response = await fetch(`${baseUrl}${route}`, {
    method,
    headers: body === null ? {} : { "content-type": "application/json" },
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
    ok: response.ok,
    status: response.status,
    body: parsed,
  }
}

async function waitForLoopRuns(loopId, minRuns, timeoutSecs) {
  const deadline = Date.now() + timeoutSecs * 1000
  let latest = null
  while (Date.now() < deadline) {
    latest = await api("GET", `/api/loops/${encodeURIComponent(loopId)}`)
    const runs = Array.isArray(latest?.runs) ? latest.runs : []
    if (runs.length >= minRuns) return latest
    await sleep(750)
  }
  throw new Error(`Loop ${loopId} did not record ${minRuns} run(s) within ${timeoutSecs}s`)
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

function appendArtifact({ status, setup, verify, state, error }) {
  fs.mkdirSync(path.dirname(artifactPath), { recursive: true })
  const finishedAt = new Date().toISOString()
  const lines = [
    "",
    `## Matrix Runner - ${finishedAt}`,
    "",
    `- Status: \`${status}\``,
    `- Scenario: \`${args.scenario}\``,
    `- Run id: \`${runId}\``,
    `- Base URL: \`${baseUrl}\``,
    `- Data dir: \`${dataDir}\``,
    `- State file: \`${stateFile}\``,
    `- Kill timing: \`${args.killDelaySecs}s after setup returned\``,
    `- Restart delay: \`${args.restartDelaySecs}s\``,
    `- Session id: \`${state?.sessionId ?? ""}\``,
    `- Goal id: \`${state?.goalId ?? ""}\``,
    `- Loop id: \`${state?.loopId ?? ""}\``,
    `- Workflow run id: \`${state?.workflowRunId ?? ""}\``,
    "",
    status === "completed"
      ? "This runner intentionally records a real process kill/restart sample. It does not check coverage boxes or mark the strict proof as passed; a reviewer must compare setup/verify observations and decide whether the sample satisfies the required coverage."
      : `Runner failed before completing the matrix sample: ${error}`,
    "",
    "```json",
    JSON.stringify(
      {
        startedAt,
        finishedAt,
        events,
        sample: arguments[0].sample ?? null,
        setup: setup ? summarizeProbe(setup) : null,
        verify: verify ? summarizeProbe(verify) : null,
      },
      null,
      2,
    ),
    "```",
    "",
  ]
  fs.appendFileSync(artifactPath, `${lines.join("\n")}`)
}

function summarizeLoopSnapshot(snapshot) {
  const schedule = snapshot?.schedule ?? snapshot
  const runs = Array.isArray(snapshot?.runs) ? snapshot.runs : []
  if (!schedule) return null
  return {
    id: schedule.id ?? null,
    sessionId: schedule.sessionId ?? schedule.session_id ?? null,
    state: schedule.state ?? null,
    triggerKind: schedule.triggerKind ?? schedule.trigger_kind ?? null,
    executionStrategy: schedule.executionStrategy ?? schedule.execution_strategy ?? null,
    cronStatus: schedule.cronStatus ?? schedule.cron_status ?? null,
    runs: runs.map((run) => ({
      id: run.id,
      state: run.state,
      seq: run.seq,
      triggerReason: run.triggerReason,
      schedulingDecision: run.schedulingDecision,
      progressState: run.progressState,
      noProgressReason: run.noProgressReason,
      startedAt: run.startedAt,
      finishedAt: run.finishedAt,
    })),
    runCount: runs.length,
    nextRunAt: schedule.nextRunAt ?? schedule.next_run_at ?? null,
  }
}

function summarizeProbe(result) {
  return {
    phase: result?.phase,
    startedAt: result?.startedAt,
    finishedAt: result?.finishedAt,
    sessionId: result?.state?.sessionId,
    goalId: result?.state?.goalId,
    loopId: result?.state?.loopId,
    workflowRunId: result?.state?.workflowRunId,
    observations: result?.observations?.map((event) => ({
      name: event.name,
      value: event.value,
    })),
  }
}

function summarizeState(state) {
  return {
    sessionId: state?.sessionId,
    goalId: state?.goalId,
    loopId: state?.loopId,
    workflowRunId: state?.workflowRunId,
    scenario: state?.scenario,
    setupAt: state?.setupAt,
    lastVerifiedAt: state?.lastVerifiedAt,
    attempts: state?.attempts?.map((attempt) => ({
      name: attempt.name,
      ok: attempt.ok,
      status: attempt.status,
    })),
  }
}

function record(name, value) {
  events.push({ at: new Date().toISOString(), name, value })
}

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"))
}

function writeJson(file, value) {
  fs.mkdirSync(path.dirname(file), { recursive: true })
  fs.writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`)
}

function safeReadJson(file) {
  if (!fs.existsSync(file)) return null
  try {
    return readJson(file)
  } catch {
    return null
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
    dataDir: null,
    exerciseControls: false,
    help: false,
    identifier: null,
    json: false,
    dynamicLoopFallbackSecs: 120,
    killDelaySecs: 1,
    longSleepSecs: 180,
    plansDir: null,
    restartDelaySecs: 2,
    runId: null,
    scenario: "long-workflow",
    serverPort: 18430,
    stateFile: null,
    vitePort: 1430,
    waitHealthSecs: 60,
    waitStateSecs: 20,
  }

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === "--artifact") parsed.artifact = argv[++i]
    else if (arg === "--data-dir") parsed.dataDir = argv[++i]
    else if (arg === "--exercise-controls") parsed.exerciseControls = true
    else if (arg === "--help" || arg === "-h") parsed.help = true
    else if (arg === "--identifier") parsed.identifier = argv[++i]
    else if (arg === "--json") parsed.json = true
    else if (arg === "--dynamic-loop-fallback-secs") parsed.dynamicLoopFallbackSecs = parsePositiveInt(argv[++i], "--dynamic-loop-fallback-secs")
    else if (arg === "--kill-delay-secs") parsed.killDelaySecs = parsePositiveInt(argv[++i], "--kill-delay-secs")
    else if (arg === "--long-sleep-secs") parsed.longSleepSecs = parsePositiveInt(argv[++i], "--long-sleep-secs")
    else if (arg === "--plans-dir") parsed.plansDir = argv[++i]
    else if (arg === "--restart-delay-secs") parsed.restartDelaySecs = parsePositiveInt(argv[++i], "--restart-delay-secs")
    else if (arg === "--run-id") parsed.runId = argv[++i]
    else if (arg === "--scenario") parsed.scenario = argv[++i]
    else if (arg === "--server-port") parsed.serverPort = parsePort(argv[++i], "--server-port")
    else if (arg === "--state-file") parsed.stateFile = argv[++i]
    else if (arg === "--vite-port") parsed.vitePort = parsePort(argv[++i], "--vite-port")
    else if (arg === "--wait-health-secs") parsed.waitHealthSecs = parsePositiveInt(argv[++i], "--wait-health-secs")
    else if (arg === "--wait-state-secs") parsed.waitStateSecs = parsePositiveInt(argv[++i], "--wait-state-secs")
    else throw new Error(`Unknown argument: ${arg}`)
  }

  if (!["long-workflow", "dynamic-loop", "incognito-guard"].includes(parsed.scenario)) {
    throw new Error("--scenario currently supports: long-workflow, dynamic-loop, incognito-guard")
  }
  return parsed
}

function printHelp() {
  process.stdout.write(`Usage:
  node scripts/v3-restart-resume-matrix-runner.mjs

Purpose:
  Run a real cross-process restart sample for the V3 restart/resume matrix.
  The runner starts an isolated Tauri desktop instance, uses the existing probe
  to create a long running Workflow/background job, kills the isolated process
  group while work is in progress, restarts the same HA_DATA_DIR and ports, and
  verifies durable owner-API state after restart.

Important:
  This script records evidence only. It never checks coverage boxes and never
  marks strict proof as passed.

Options:
  --data-dir <path>             Reusable isolated HA_DATA_DIR.
  --state-file <path>           Probe state JSON. Defaults to <data-dir>/probe-state.json.
  --artifact <path>             Evidence artifact to append.
  --plans-dir <path>            Override Plans directory.
  --scenario <name>             long-workflow | dynamic-loop | incognito-guard.
  --long-sleep-secs <seconds>   Long validation sleep. Defaults to 180.
  --dynamic-loop-fallback-secs <seconds>
                                Dynamic Loop fallback interval. Defaults to 120.
  --kill-delay-secs <seconds>   Delay after setup returns before killing. Defaults to 1.
  --restart-delay-secs <secs>   Delay before restart. Defaults to 2.
  --vite-port <port>            Preferred Vite port. Defaults to 1430.
  --server-port <port>          Preferred embedded server port. Defaults to 18430.
  --identifier <id>             Tauri app identifier.
  --run-id <id>                 Stable label for logs/data dir.
  --json                        Print full machine-readable result.
  --help, -h                    Show this help.
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
      "V3 restart/resume matrix runner completed.",
      `Run id: ${result.runId}`,
      `Scenario: ${result.scenario}`,
      `Data dir: ${result.dataDir}`,
      `State file: ${result.stateFile}`,
      `Artifact: ${result.artifactPath}`,
      `Base URL: ${result.baseUrl}`,
      "",
    ].join("\n"),
  )
}
