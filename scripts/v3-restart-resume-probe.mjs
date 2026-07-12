#!/usr/bin/env node
import fs from "node:fs"
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

const plansDir = path.resolve(args.plansDir ?? process.env.HOPE_V3_PLANS_DIR ?? defaultPlansDir)
const defaultArtifact = path.join(plansDir, "evidence/restart-resume-matrix-2026-07-08.md")
const defaultStateFile = path.join(plansDir, "evidence/restart-resume-probe-state.json")
const artifactPath = path.resolve(args.artifact ?? defaultArtifact)
const stateFile = path.resolve(args.stateFile ?? defaultStateFile)
const baseUrl = args.baseUrl.replace(/\/+$/, "")

const phase = args.phase ?? "setup"
if (!["setup", "verify", "snapshot"].includes(phase)) {
  fail('--phase must be one of: setup, verify, snapshot')
}
if (!["basic", "approval-waiting", "long-workflow"].includes(args.scenario)) {
  fail('--scenario must be one of: basic, approval-waiting, long-workflow')
}

const startedAt = new Date().toISOString()
const observations = []

await waitForHealth()

let state = readStateFile(stateFile)
if (phase === "setup") {
  state = await runSetup(state)
  writeJson(stateFile, state)
} else {
  if (args.sessionId) state.sessionId = args.sessionId
  if (!state.sessionId) {
    fail(`No session id. Pass --session-id or run setup first. State file: ${stateFile}`)
  }
  state = await runVerify(state, { exerciseControls: args.exerciseControls })
  writeJson(stateFile, state)
}

const result = {
  phase,
  baseUrl,
  stateFile,
  artifactPath,
  startedAt,
  finishedAt: new Date().toISOString(),
  state,
  observations,
  next: nextStepsFor(phase),
}

if (!args.noWriteArtifact) appendArtifact(result)

if (args.json) {
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`)
} else {
  printHuman(result)
}

async function runSetup(existing) {
  const session = existing.sessionId
    ? await api("GET", `/api/sessions/${encodeURIComponent(existing.sessionId)}`)
    : await api("POST", "/api/sessions", { incognito: false })
  const sessionId = session.id
  record("session", { id: sessionId, title: session.title ?? null })

  const goal = existing.goalId
    ? await api("GET", `/api/goals/${encodeURIComponent(existing.goalId)}`)
    : await api("POST", `/api/sessions/${encodeURIComponent(sessionId)}/goal`, {
        objective:
          "V3 restart/resume probe: keep this durable Goal active across a real app restart.",
        completionCriteria:
          "[required] Goal remains visible after restart.\n[required] Goal can be paused/resumed through owner API after restart.\n[required] Watchdog status can be read after restart.",
        domain: "general",
        budgetTurnLimit: 4,
      })
  const goalSnapshot = goal && goal.goal ? goal : await api("GET", `/api/goals/${encodeURIComponent(goal.id)}`)
  record("goal_created_or_loaded", summarizeGoal(goalSnapshot))

  const loop = existing.loopId
    ? await api("GET", `/api/loops/${encodeURIComponent(existing.loopId)}`)
    : await api("POST", `/api/sessions/${encodeURIComponent(sessionId)}/loops`, {
        triggerKind: "dynamic",
        triggerSpec: {
          fallbackSecs: args.loopFallbackSecs,
          fallbackUsed: false,
        },
        executionStrategy: "continue",
        prompt:
          "V3 restart/resume probe loop: confirm durable schedule, history, and watchdog visibility after restart.",
        goalId: goalSnapshot.goal?.id ?? goalSnapshot.id,
        maxRuns: 3,
        maxRuntimeSecs: 3600,
        tokenBudget: 20000,
        maxNoProgressRuns: 2,
        maxFailures: 2,
        backoffSecs: 300,
      })
  const loopSnapshot = loop && loop.schedule ? loop : await api("GET", `/api/loops/${encodeURIComponent(loop.id)}`)
  record("loop_created_or_loaded", summarizeLoop(loopSnapshot))

  const workflow = existing.workflowRunId
    ? await api("GET", `/api/workflow-runs/${encodeURIComponent(existing.workflowRunId)}`)
    : await api("POST", `/api/sessions/${encodeURIComponent(sessionId)}/workflow-runs`, {
        kind: "general.workflow",
        executionMode: "guarded",
        scriptSource: workflowScriptForScenario(args.scenario),
        origin: "strict-proof:restart-resume-probe",
        goalId: goalSnapshot.goal?.id ?? goalSnapshot.id,
        runImmediately: shouldRunWorkflowImmediately(),
        budget: {
          maxOps: 12,
          maxRuntimeSecs: Math.max(300, args.longSleepSecs + 60),
        },
      })
  if (!existing.workflowRunId) {
    await waitForScenarioSetupState(workflow.id)
  }
  const workflowSnapshot =
    workflow && workflow.run ? workflow : await api("GET", `/api/workflow-runs/${encodeURIComponent(workflow.id)}`)
  record("workflow_created_or_loaded", summarizeWorkflow(workflowSnapshot))

  const probes = await collectReadOnlyProbes(sessionId, {
    goalId: goalSnapshot.goal?.id ?? goalSnapshot.id,
    loopId: loopSnapshot.schedule?.id ?? loopSnapshot.id,
    workflowRunId: workflowSnapshot.run?.id ?? workflowSnapshot.id,
  })

  return {
    ...existing,
    sessionId,
    goalId: goalSnapshot.goal?.id ?? goalSnapshot.id,
    loopId: loopSnapshot.schedule?.id ?? loopSnapshot.id,
    workflowRunId: workflowSnapshot.run?.id ?? workflowSnapshot.id,
    scenario: args.scenario,
    setupAt: startedAt,
    lastVerifiedAt: null,
    probes,
  }
}

async function runVerify(existing, { exerciseControls }) {
  const probes = await collectReadOnlyProbes(existing.sessionId, existing)
  if (exerciseControls) {
    if (existing.goalId) {
      const paused = await api("POST", `/api/goals/${encodeURIComponent(existing.goalId)}/pause`)
      record("goal_pause_after_restart", summarizeGoal(paused))
      const resumed = await api("POST", `/api/goals/${encodeURIComponent(existing.goalId)}/resume`)
      record("goal_resume_after_restart", summarizeGoal(resumed))
    }
    if (existing.loopId) {
      const paused = await api("POST", `/api/loops/${encodeURIComponent(existing.loopId)}/pause`)
      record("loop_pause_after_restart", summarizeLoop(paused))
      const resumed = await api("POST", `/api/loops/${encodeURIComponent(existing.loopId)}/resume`)
      record("loop_resume_after_restart", summarizeLoop(resumed))
    }
  }
  return {
    ...existing,
    lastVerifiedAt: startedAt,
    probes,
  }
}

async function collectReadOnlyProbes(sessionId, ids) {
  const probes = {}
  probes.health = await api("GET", "/api/health", null, { public: true })
  record("health", probes.health)
  probes.session = await api("GET", `/api/sessions/${encodeURIComponent(sessionId)}`)
  record("session_after_read", { id: probes.session.id, title: probes.session.title ?? null })
  probes.activeGoal = await api("GET", `/api/sessions/${encodeURIComponent(sessionId)}/goal`)
  record("active_goal", summarizeGoal(probes.activeGoal))
  probes.goalWatchdog = await api("GET", `/api/sessions/${encodeURIComponent(sessionId)}/goal/watchdog?staleSecs=1`)
  record("goal_watchdog", summarizeArray(probes.goalWatchdog))
  probes.loops = await api("GET", `/api/sessions/${encodeURIComponent(sessionId)}/loops`)
  record("loops", summarizeArray(probes.loops))
  probes.loopWatchdog = await api("GET", `/api/sessions/${encodeURIComponent(sessionId)}/loops/watchdog?graceSecs=1`)
  record("loop_watchdog", summarizeArray(probes.loopWatchdog))
  probes.workflowRuns = await api("GET", `/api/sessions/${encodeURIComponent(sessionId)}/workflow-runs`)
  record("workflow_runs", summarizeArray(probes.workflowRuns))
  probes.workflowWatchdog = await api(
    "GET",
    `/api/sessions/${encodeURIComponent(sessionId)}/workflow-runs/watchdog?staleSecs=1`,
  )
  record("workflow_watchdog", summarizeArray(probes.workflowWatchdog))
  probes.backgroundJobs = await api("GET", `/api/sessions/${encodeURIComponent(sessionId)}/background-jobs`)
  record("background_jobs", summarizeArray(probes.backgroundJobs))

  if (ids.goalId) {
    probes.goal = await api("GET", `/api/goals/${encodeURIComponent(ids.goalId)}`)
    record("goal_by_id", summarizeGoal(probes.goal))
  }
  if (ids.loopId) {
    probes.loop = await api("GET", `/api/loops/${encodeURIComponent(ids.loopId)}`)
    record("loop_by_id", summarizeLoop(probes.loop))
  }
  if (ids.workflowRunId) {
    probes.workflowRun = await api("GET", `/api/workflow-runs/${encodeURIComponent(ids.workflowRunId)}`)
    record("workflow_by_id", summarizeWorkflow(probes.workflowRun))
  }
  return probes
}

async function waitForScenarioSetupState(runId) {
  if (args.scenario === "basic" && !args.runWorkflow) return
  const deadline = Date.now() + args.waitStateSecs * 1000
  let lastSnapshot = null
  while (Date.now() < deadline) {
    lastSnapshot = await api("GET", `/api/workflow-runs/${encodeURIComponent(runId)}`)
    const state = lastSnapshot?.run?.state ?? lastSnapshot?.state
    if (args.scenario === "approval-waiting" && state === "awaiting_approval") {
      record("scenario_state_ready", { scenario: args.scenario, runId, state })
      return
    }
    if (args.scenario === "long-workflow" && ["running", "recovering"].includes(state)) {
      const jobs = await api("GET", `/api/sessions/${encodeURIComponent(lastSnapshot.run.sessionId)}/background-jobs`)
      if (jobs.some((job) => ["running", "awaiting_approval", "queued"].includes(job.status))) {
        record("scenario_state_ready", {
          scenario: args.scenario,
          runId,
          state,
          activeJobs: jobs
            .filter((job) => ["running", "awaiting_approval", "queued"].includes(job.status))
            .map((job) => ({ jobId: job.jobId, status: job.status, tool: job.toolName })),
        })
        return
      }
    }
    if (args.scenario === "basic" && args.runWorkflow && ["running", "completed", "awaiting_approval"].includes(state)) {
      record("scenario_state_ready", { scenario: args.scenario, runId, state })
      return
    }
    await sleep(500)
  }
  record("scenario_state_timeout", {
    scenario: args.scenario,
    runId,
    last: summarizeWorkflow(lastSnapshot),
  })
}

function shouldRunWorkflowImmediately() {
  return args.runWorkflow || args.scenario === "approval-waiting" || args.scenario === "long-workflow"
}

function workflowScriptForScenario(scenario) {
  if (scenario === "approval-waiting") return approvalWaitingWorkflowScript()
  if (scenario === "long-workflow") return longWorkflowScript()
  return basicWorkflowScript()
}

function basicWorkflowScript() {
  return `export default async function main(workflow) {
  const task = await workflow.task.create({ title: "V3 restart/resume probe workflow" });
  await workflow.trace({
    label: "probe-start",
    payload: { source: "v3-restart-resume-probe", expectation: "durable across restart" }
  });
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({
    summary: "V3 restart/resume probe workflow completed.",
    source: "v3-restart-resume-probe"
  });
}
`
}

function approvalWaitingWorkflowScript() {
  return `export default async function main(workflow) {
  const task = await workflow.task.create({ title: "V3 restart/resume approval waiting probe" });
  await workflow.tool({
    name: "write",
    args: {
      path: "v3-restart-approval-probe.txt",
      content: "This file should not be written before explicit workflow approval."
    },
    label: "approval-required-write"
  });
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({
    summary: "V3 approval waiting restart probe completed.",
    source: "v3-restart-resume-probe"
  });
}
`
}

function longWorkflowScript() {
  return `export default async function main(workflow) {
  const task = await workflow.task.create({ title: "V3 restart/resume long validation probe" });
  await workflow.trace({
    label: "long-validation-start",
    payload: { source: "v3-restart-resume-probe", sleepSecs: ${args.longSleepSecs} }
  });
  const validation = await workflow.validate({
    reason: "V3 restart/resume running workflow and background job sample",
    commands: [{ command: "sleep ${args.longSleepSecs}; echo v3-long-workflow-finished", timeout: ${args.longSleepSecs + 30} }]
  });
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({
    summary: "V3 long workflow restart probe completed.",
    validation,
    source: "v3-restart-resume-probe"
  });
}
`
}

async function waitForHealth() {
  const deadline = Date.now() + args.waitHealthSecs * 1000
  let lastError = null
  while (Date.now() < deadline) {
    try {
      const health = await api("GET", "/api/health", null, { public: true })
      record("health_ready", health)
      return
    } catch (error) {
      lastError = error
      await sleep(750)
    }
  }
  fail(`Health check did not become ready within ${args.waitHealthSecs}s: ${lastError?.message ?? "unknown error"}`)
}

async function api(method, route, body = null, options = {}) {
  const headers = {}
  if (body !== null) headers["content-type"] = "application/json"
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
  if (!response.ok) {
    throw new Error(`${method} ${route} -> ${response.status}: ${typeof parsed === "string" ? parsed : JSON.stringify(parsed)}`)
  }
  return parsed
}

function record(name, value) {
  observations.push({ at: new Date().toISOString(), name, value })
}

function summarizeGoal(value) {
  const goal = value?.goal ?? value
  if (!goal) return null
  return {
    id: goal.id ?? null,
    sessionId: goal.sessionId ?? goal.session_id ?? null,
    state: goal.state ?? null,
    revision: goal.revision ?? null,
    closureDecision: goal.closureDecision ?? goal.closure_decision ?? null,
    tasks: value?.tasks?.length ?? goal.tasks?.length ?? null,
    evidence: value?.evidence?.length ?? goal.evidence?.length ?? null,
  }
}

function summarizeLoop(value) {
  const schedule = value?.schedule ?? value
  if (!schedule) return null
  return {
    id: schedule.id ?? null,
    sessionId: schedule.sessionId ?? schedule.session_id ?? null,
    state: schedule.state ?? null,
    triggerKind: schedule.triggerKind ?? schedule.trigger_kind ?? null,
    executionStrategy: schedule.executionStrategy ?? schedule.execution_strategy ?? null,
    cronStatus: schedule.cronStatus ?? schedule.cron_status ?? null,
    runs: value?.runs?.length ?? schedule.runs?.length ?? null,
    nextRunAt: schedule.nextRunAt ?? schedule.next_run_at ?? null,
  }
}

function summarizeWorkflow(value) {
  const run = value?.run ?? value
  if (!run) return null
  return {
    id: run.id ?? null,
    sessionId: run.sessionId ?? run.session_id ?? null,
    state: run.state ?? null,
    kind: run.kind ?? null,
    executionMode: run.executionMode ?? run.execution_mode ?? null,
    origin: run.origin ?? null,
    ops: value?.ops?.length ?? run.ops?.length ?? null,
    events: value?.events?.length ?? run.events?.length ?? null,
  }
}

function summarizeArray(value) {
  if (!Array.isArray(value)) return value
  return {
    count: value.length,
    sample: value.slice(0, 3).map((item) => ({
      id: item?.id ?? item?.runId ?? item?.loopId ?? item?.goalId ?? null,
      state: item?.state ?? item?.status ?? item?.kind ?? null,
      reason: item?.reason ?? item?.message ?? item?.title ?? null,
    })),
  }
}

function appendArtifact(result) {
  fs.mkdirSync(path.dirname(artifactPath), { recursive: true })
  const lines = [
    "",
    `## Probe Observations - ${result.finishedAt}`,
    "",
    `- Phase: \`${result.phase}\``,
    `- Scenario: \`${result.state.scenario ?? args.scenario}\``,
    `- Base URL: \`${result.baseUrl}\``,
    `- State file: \`${result.stateFile}\``,
    `- Session id: \`${result.state.sessionId ?? ""}\``,
    `- Goal id: \`${result.state.goalId ?? ""}\``,
    `- Loop id: \`${result.state.loopId ?? ""}\``,
    `- Workflow run id: \`${result.state.workflowRunId ?? ""}\``,
    "",
    "```json",
    JSON.stringify(
      {
        phase: result.phase,
        startedAt: result.startedAt,
        finishedAt: result.finishedAt,
        observations: result.observations,
        next: result.next,
      },
      null,
      2,
    ),
    "```",
    "",
  ]
  fs.appendFileSync(artifactPath, `${lines.join("\n")}`)
}

function printHuman(result) {
  process.stdout.write(
    [
      `V3 restart/resume probe ${result.phase} completed.`,
      `Base URL: ${result.baseUrl}`,
      `State file: ${result.stateFile}`,
      `Artifact: ${args.noWriteArtifact ? "(not written)" : result.artifactPath}`,
      `Session: ${result.state.sessionId ?? "(none)"}`,
      `Goal: ${result.state.goalId ?? "(none)"}`,
      `Loop: ${result.state.loopId ?? "(none)"}`,
    `Workflow run: ${result.state.workflowRunId ?? "(none)"}`,
      `Scenario: ${result.state.scenario ?? args.scenario}`,
      "",
      "Next:",
      ...result.next.map((line) => `- ${line}`),
      "",
    ].join("\n"),
  )
}

function nextStepsFor(currentPhase) {
  if (currentPhase === "setup") {
    return [
      "Quit or kill the isolated Hope Agent desktop/server process.",
      "Restart the same HA_DATA_DIR and embedded server port.",
      `Run: node scripts/v3-restart-resume-probe.mjs --phase verify --base-url ${baseUrl} --state-file ${shellQuote(stateFile)}`,
      "Use the appended artifact observations as input; do not mark coverage passed until the real restart and manual review are complete.",
    ]
  }
  return [
    "Compare setup and verify observations in the artifact.",
    "Manually complete the restart matrix coverage boxes only after running the missing workflow/approval/background-job scenarios.",
    "Then mark the manifest entry passed with scripts/v3-strict-proof-record.mjs --status passed --confirm-reviewed.",
  ]
}

function readStateFile(file) {
  if (!fs.existsSync(file)) return {}
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"))
  } catch (error) {
    fail(`Failed to read state file ${file}: ${error.message}`)
  }
}

function writeJson(file, value) {
  fs.mkdirSync(path.dirname(file), { recursive: true })
  fs.writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`)
}

function parseArgs(argv) {
  const parsed = {
    artifact: null,
    baseUrl: "http://127.0.0.1:18421",
    exerciseControls: false,
    help: false,
    json: false,
    loopFallbackSecs: 1200,
    longSleepSecs: 120,
    noWriteArtifact: false,
    phase: null,
    plansDir: null,
    runWorkflow: false,
    scenario: "basic",
    sessionId: null,
    stateFile: null,
    token: process.env.HOPE_API_TOKEN ?? null,
    waitHealthSecs: 30,
    waitStateSecs: 15,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === "--artifact") parsed.artifact = argv[++i]
    else if (arg === "--base-url") parsed.baseUrl = argv[++i]
    else if (arg === "--exercise-controls") parsed.exerciseControls = true
    else if (arg === "--help" || arg === "-h") parsed.help = true
    else if (arg === "--json") parsed.json = true
    else if (arg === "--loop-fallback-secs") parsed.loopFallbackSecs = parsePositiveInt(argv[++i], "--loop-fallback-secs")
    else if (arg === "--long-sleep-secs") parsed.longSleepSecs = parsePositiveInt(argv[++i], "--long-sleep-secs")
    else if (arg === "--no-write-artifact") parsed.noWriteArtifact = true
    else if (arg === "--phase") parsed.phase = argv[++i]
    else if (arg === "--plans-dir") parsed.plansDir = argv[++i]
    else if (arg === "--run-workflow") parsed.runWorkflow = true
    else if (arg === "--scenario") parsed.scenario = argv[++i]
    else if (arg === "--session-id") parsed.sessionId = argv[++i]
    else if (arg === "--state-file") parsed.stateFile = argv[++i]
    else if (arg === "--token") parsed.token = argv[++i]
    else if (arg === "--wait-health-secs") parsed.waitHealthSecs = parsePositiveInt(argv[++i], "--wait-health-secs")
    else if (arg === "--wait-state-secs") parsed.waitStateSecs = parsePositiveInt(argv[++i], "--wait-state-secs")
    else fail(`Unknown argument: ${arg}`)
  }
  return parsed
}

function printHelp() {
  process.stdout.write(`Usage:
  node scripts/v3-restart-resume-probe.mjs --phase setup --base-url http://127.0.0.1:18421
  node scripts/v3-restart-resume-probe.mjs --phase verify --base-url http://127.0.0.1:18421

Purpose:
  Collect real owner-API observations for the V3 restart/resume strict proof.
  The script creates or reads durable Goal, Loop, and Workflow records, then
  appends before/after observations to the restart-resume artifact. It does not
  mark strict proof coverage as passed; a human reviewer must still run the
  real restart and complete the checklist.

Recommended flow:
  1. Launch an isolated desktop instance:
     node scripts/v3-tauri-smoke-launch.mjs --run
  2. In another terminal, run setup with the script's health/base URL.
  3. Quit or kill the app process.
  4. Restart the same HA_DATA_DIR/port.
  5. Run verify with the same --state-file.

Options:
  --phase setup|verify|snapshot   Defaults to setup.
  --base-url <url>                Owner API base URL. Defaults to http://127.0.0.1:18421.
  --token <token>                 Optional Bearer token; also reads HOPE_API_TOKEN.
  --state-file <path>             Probe state JSON. Defaults to Plans/evidence/restart-resume-probe-state.json.
  --artifact <path>               Artifact to append. Defaults to Plans/evidence/restart-resume-matrix-2026-07-08.md.
  --plans-dir <path>              Override Plans directory.
  --session-id <id>               Reuse an existing session.
  --scenario <name>               basic | approval-waiting | long-workflow. Defaults to basic.
  --run-workflow                  During setup, immediately launch the probe workflow.
  --long-sleep-secs <seconds>     Sleep duration for long-workflow. Defaults to 120.
  --exercise-controls             During verify, pause/resume Goal and Loop through owner API.
  --loop-fallback-secs <seconds>  Dynamic Loop fallback interval. Defaults to 1200.
  --wait-health-secs <seconds>    Health wait timeout. Defaults to 30.
  --wait-state-secs <seconds>     Scenario setup state wait timeout. Defaults to 15.
  --no-write-artifact             Do not append observations to the artifact.
  --json                          Print full machine-readable result.
  --help, -h                      Show this help.
`)
}

function parsePositiveInt(value, flag) {
  const parsed = Number.parseInt(value, 10)
  if (!Number.isInteger(parsed) || parsed <= 0) fail(`${flag} must be a positive integer.`)
  return parsed
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

function shellQuote(value) {
  if (/^[A-Za-z0-9_./:=@+-]+$/.test(value)) return value
  return `'${value.replaceAll("'", "'\\''")}'`
}

function fail(message) {
  process.stderr.write(`${message}\n`)
  process.exit(1)
}
