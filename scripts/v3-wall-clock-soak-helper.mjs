#!/usr/bin/env node
import childProcess from "node:child_process"
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

const repoRoot = process.cwd()
const plansDir = path.resolve(args.plansDir ?? process.env.HOPE_V3_PLANS_DIR ?? defaultPlansDir)
const artifactRel = args.artifact ?? "evidence/wall-clock-soak-2026-07-08.md"
const artifactPath = path.resolve(plansDir, artifactRel)
const timestamp = new Date().toISOString()
const timezone = Intl.DateTimeFormat().resolvedOptions().timeZone || "unknown"
const phase = args.phase ?? "start"
if (!["start", "checkpoint", "finish"].includes(phase)) {
  fail("--phase must be one of: start, checkpoint, finish")
}

const packageJson = readJson(path.join(repoRoot, "package.json"))
const context = {
  artifactPath,
  artifactRel,
  branch: runGit(["branch", "--show-current"]).trim() || "unknown",
  commit: runGit(["rev-parse", "--short=9", "HEAD"]).trim() || "unknown",
  ids: {
    goalId: args.goalId,
    loopId: args.loopId,
    loopRunId: args.loopRunId,
    sessionId: args.sessionId,
    workflowRunId: args.workflowRunId,
  },
  notes: args.notes ?? "",
  packageVersion: packageJson.version ?? "unknown",
  phase,
  plansDir,
  repoRoot,
  reviewer: args.reviewer ?? "manual:<name>",
  startAt: args.startAt,
  timestamp,
  timezone,
}

const launch = phase === "start" ? prepareLaunch(args) : null
const packet =
  phase === "start"
    ? renderStartPacket({ ...context, launch })
    : phase === "checkpoint"
      ? renderCheckpointPacket(context)
      : renderFinishPacket(context)

if (args.append) {
  fs.mkdirSync(path.dirname(artifactPath), { recursive: true })
  fs.appendFileSync(artifactPath, `\n${packet}\n`)
}

if (args.json) {
  process.stdout.write(
    `${JSON.stringify(
      {
        artifactPath,
        appended: args.append,
        elapsed: elapsedSummary(args.startAt, timestamp),
        ids: context.ids,
        launch:
          launch === null
            ? null
            : {
                dataDir: launch.dataDir,
                helperLaunchCommand: launch.helperLaunchCommand,
                summary: launch.summary,
              },
        phase,
        plansDir,
        timestamp,
        timezone,
      },
      null,
      2,
    )}\n`,
  )
} else {
  process.stdout.write(packet)
  process.stdout.write("\n")
  if (args.append) process.stdout.write(`Appended soak ${phase} packet to: ${artifactPath}\n`)
}

function parseArgs(argv) {
  const parsed = {
    append: false,
    artifact: null,
    dataDir: null,
    force: false,
    goalId: null,
    help: false,
    identifier: null,
    json: false,
    loopId: null,
    loopRunId: null,
    notes: null,
    phase: null,
    plansDir: null,
    reviewer: null,
    serverPort: 18421,
    sessionId: null,
    startAt: null,
    vitePort: 1422,
    workflowRunId: null,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === "--append") parsed.append = true
    else if (arg === "--artifact") parsed.artifact = argv[++i]
    else if (arg === "--data-dir") parsed.dataDir = argv[++i]
    else if (arg === "--force") parsed.force = true
    else if (arg === "--goal-id") parsed.goalId = argv[++i]
    else if (arg === "--help" || arg === "-h") parsed.help = true
    else if (arg === "--identifier") parsed.identifier = argv[++i]
    else if (arg === "--json") parsed.json = true
    else if (arg === "--loop-id") parsed.loopId = argv[++i]
    else if (arg === "--loop-run-id") parsed.loopRunId = argv[++i]
    else if (arg === "--notes") parsed.notes = argv[++i]
    else if (arg === "--phase") parsed.phase = argv[++i]
    else if (arg === "--plans-dir") parsed.plansDir = argv[++i]
    else if (arg === "--reviewer") parsed.reviewer = argv[++i]
    else if (arg === "--server-port") parsed.serverPort = parsePort(argv[++i], "--server-port")
    else if (arg === "--session-id") parsed.sessionId = argv[++i]
    else if (arg === "--start-at") parsed.startAt = argv[++i]
    else if (arg === "--vite-port") parsed.vitePort = parsePort(argv[++i], "--vite-port")
    else if (arg === "--workflow-run-id") parsed.workflowRunId = argv[++i]
    else fail(`Unknown argument: ${arg}`)
  }
  return parsed
}

function printHelp() {
  process.stdout.write(`Usage:
  node scripts/v3-wall-clock-soak-helper.mjs --phase start [options]
  node scripts/v3-wall-clock-soak-helper.mjs --phase checkpoint --start-at <iso> [ids/options]
  node scripts/v3-wall-clock-soak-helper.mjs --phase finish --start-at <iso> [ids/options]

Purpose:
  Create real wall-clock soak execution packets. The helper records actual ISO
  timestamps, timezone, prompts, ids, and reviewer commands. It never checks
  coverage boxes and never marks strict proof passed.

Options:
  --append                   Append the packet to the wall-clock soak artifact.
  --json                     Print machine-readable metadata.
  --phase <name>             start | checkpoint | finish. Defaults to start.
  --plans-dir <path>         Override the V3 Plans directory.
  --artifact <path>          Artifact path relative to the Plans directory.
  --reviewer <name>          Reviewer label for the packet/pass command.
  --start-at <iso>           Soak start timestamp for checkpoint/finish elapsed.
  --session-id <id>          Durable session id.
  --goal-id <id>             Durable Goal id.
  --loop-id <id>             Durable Loop id.
  --loop-run-id <id>         Durable Loop run id.
  --workflow-run-id <id>     Durable Workflow run id.
  --notes <text>             Manual observation notes.
  --data-dir <path>          Isolated Tauri data dir for start packet.
  --force                    Allow reusing an existing data dir for launcher prep.
  --identifier <id>          Unique Tauri app identifier.
  --vite-port <port>         Preferred Vite dev port. Defaults to 1422.
  --server-port <port>       Preferred embedded server port. Defaults to 18421.
  --help, -h                 Show this help.
`)
}

function prepareLaunch(parsed) {
  const safeTimestamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\..+/, "")
  const dataDir = path.resolve(
    parsed.dataDir ?? path.join(os.tmpdir(), `hope-agent-v3-wall-clock-soak-${safeTimestamp}`),
  )
  const launcherArgs = [
    "scripts/v3-tauri-smoke-launch.mjs",
    "--json",
    "--data-dir",
    dataDir,
    "--vite-port",
    String(parsed.vitePort),
    "--server-port",
    String(parsed.serverPort),
    "--identifier",
    parsed.identifier ?? `ai.hopeagent.desktop.v3soak.${process.pid}`,
  ]
  if (parsed.force) launcherArgs.push("--force")
  const summary = JSON.parse(runNode(launcherArgs))
  return {
    dataDir,
    helperLaunchCommand: buildLaunchCommand({
      dataDir,
      identifier: summary.identifier,
      serverPort: extractPort(summary.serverHealthUrl),
      vitePort: extractPort(summary.viteUrl),
    }),
    summary,
  }
}

function renderStartPacket(input) {
  const { artifactPath, branch, commit, launch, packageVersion, plansDir, reviewer, repoRoot, timestamp, timezone } =
    input
  return [
    `## Wall-clock Soak Start Packet - ${timestamp}`,
    "",
    "This packet starts a real wall-clock sample. It does not prove coverage by itself.",
    "",
    "### Environment",
    "",
    `- Hope commit: \`${commit}\``,
    `- Branch: \`${branch}\``,
    `- App build/version: \`${packageVersion}\``,
    `- Workspace/worktree: \`${repoRoot}\``,
    `- Plans dir: \`${plansDir}\``,
    `- Artifact: \`${artifactPath}\``,
    `- Reviewer: \`${reviewer}\``,
    `- Wall-clock start: \`${timestamp}\``,
    `- Timezone: \`${timezone}\``,
    "",
    "### Isolated Launch",
    "",
    `- Data dir: \`${launch.dataDir}\``,
    `- Identifier: \`${launch.summary.identifier}\``,
    `- Vite URL: \`${launch.summary.viteUrl}\``,
    `- Server health URL: \`${launch.summary.serverHealthUrl}\``,
    "",
    "```bash",
    launch.helperLaunchCommand,
    "```",
    "",
    "### Goal Prompt",
    "",
    "```text",
    goalPrompt(),
    "```",
    "",
    "### Loop Prompt",
    "",
    "```text",
    loopPrompt(),
    "```",
    "",
    "### Workflow Prompt",
    "",
    "```text",
    workflowPrompt(),
    "```",
    "",
    "### Checkpoint Commands",
    "",
    "Record a midpoint checkpoint:",
    "",
    "```bash",
    `node scripts/v3-wall-clock-soak-helper.mjs --phase checkpoint --append --start-at ${shellQuote(timestamp)} --reviewer ${shellQuote(reviewer)} --session-id <id> --goal-id <id> --loop-id <id> --workflow-run-id <id> --notes "<manual observation>"`,
    "```",
    "",
    "Record the finish point:",
    "",
    "```bash",
    `node scripts/v3-wall-clock-soak-helper.mjs --phase finish --append --start-at ${shellQuote(timestamp)} --reviewer ${shellQuote(reviewer)} --session-id <id> --goal-id <id> --loop-id <id> --loop-run-id <id> --workflow-run-id <id> --notes "<final observation>"`,
    "```",
    "",
  ].join("\n")
}

function renderCheckpointPacket(input) {
  const { ids, notes, startAt, timestamp, timezone } = input
  return [
    `## Wall-clock Soak Checkpoint - ${timestamp}`,
    "",
    `- Start at: \`${startAt ?? "not recorded"}\``,
    `- Checkpoint at: \`${timestamp}\``,
    `- Elapsed: \`${elapsedSummary(startAt, timestamp).label}\``,
    `- Timezone: \`${timezone}\``,
    ...idLines(ids),
    `- Notes: ${notes || "pending"}`,
    "",
    "Checkpoint requirements:",
    "",
    "- [ ] Goal continuation is visible or a durable wait state is visible.",
    "- [ ] Loop has either a natural nextRunAt, a natural run history entry, or a clear reason it has not fired yet.",
    "- [ ] Workflow has a phase/checkpoint/result/recovery-visible state.",
    "- [ ] This checkpoint uses real wall-clock time, not edited timestamps.",
    "",
  ].join("\n")
}

function renderFinishPacket(input) {
  const { artifactRel, ids, notes, reviewer, startAt, timestamp, timezone } = input
  const elapsed = elapsedSummary(startAt, timestamp)
  const passCommand = [
    "node scripts/v3-strict-proof-record.mjs \\",
    "  --requirement real_soak \\",
    "  --id real_soak_2026_07_08 \\",
    "  --status passed \\",
    `  --artifact ${artifactRel} \\`,
    `  --reviewer ${shellQuote(reviewer)} \\`,
    '  --summary "Real wall-clock soak proof completed." \\',
    "  --confirm-reviewed",
  ].join("\n")
  return [
    `## Wall-clock Soak Finish Packet - ${timestamp}`,
    "",
    `- Start at: \`${startAt ?? "not recorded"}\``,
    `- Finish at: \`${timestamp}\``,
    `- Elapsed: \`${elapsed.label}\``,
    `- Timezone: \`${timezone}\``,
    ...idLines(ids),
    `- Notes: ${notes || "pending"}`,
    "",
    "Manual reviewer must confirm before checking coverage:",
    "",
    "- [ ] wall_clock: elapsed time is real and sufficient for the recorded sample.",
    "- [ ] goal_continuation: Goal continued across at least two turns or durable wait/resume was visible.",
    "- [ ] loop_reschedule: Loop had at least one natural scheduled trigger or natural scheduling decision; manual run-now was not the only evidence.",
    "- [ ] workflow_recovery: Workflow phase/checkpoint/result/recovery state was visible and matched durable state.",
    "- [ ] final UI state, durable store state, and model-facing state agree.",
    "",
    "Mark passed only after the artifact's Required Coverage and Reviewer Decision boxes are checked:",
    "",
    "```bash",
    passCommand,
    "```",
    "",
  ].join("\n")
}

function goalPrompt() {
  return `/goal 在当前工作区完成一轮 30 分钟级稳定性长跑验收：创建一个不修改真实业务文件的临时验证目录，持续推进一个小型可运行前端页面或文档检查任务；过程中要至少经历一次自然 Loop 触发、一次 Workflow 阶段结果或恢复可见状态，并在最终总结里说明完成标准、验证结果和剩余风险。

完成标准：
[required] 有一个明确的临时工作目录或测试文件，结束后可清理。
[required] Goal 至少经历两轮推进，不是一轮直接结束。
[required] Loop 至少发生一次自然调度触发，不只靠手动 run-now。
[required] Workflow 至少产生一个阶段性结果、checkpoint、验证结果或恢复可见状态。
[required] Workspace 中 Goal / Loop / Workflow 状态与最终消息一致。
[required] 记录本次 wall-clock 起止时间、durable ids 和任何人工干预。`
}

function loopPrompt() {
  return "/loop 每隔一段时间检查当前长跑验收是否还需要继续。若还有未完成项，请继续推进并明确下一次检查时间；若完成标准已经满足，请停止 Loop 并说明证据。"
}

function workflowPrompt() {
  return "请用工作流方式创建一个最小无副作用的长跑检查：列出当前目标、创建一个任务、等待或验证一个本地状态，然后给出阶段结果。不要访问网络，不要修改外部系统。"
}

function idLines(ids) {
  return [
    `- Session id: \`${ids.sessionId ?? "pending"}\``,
    `- Goal id: \`${ids.goalId ?? "pending"}\``,
    `- Loop id: \`${ids.loopId ?? "pending"}\``,
    `- Loop run id: \`${ids.loopRunId ?? "pending"}\``,
    `- Workflow run id: \`${ids.workflowRunId ?? "pending"}\``,
  ]
}

function elapsedSummary(startAt, endAt) {
  if (!startAt) return { label: "unknown", millis: null }
  const start = Date.parse(startAt)
  const end = Date.parse(endAt)
  if (!Number.isFinite(start) || !Number.isFinite(end) || end < start) return { label: "invalid", millis: null }
  const millis = end - start
  const totalSeconds = Math.floor(millis / 1000)
  const hours = Math.floor(totalSeconds / 3600)
  const minutes = Math.floor((totalSeconds % 3600) / 60)
  const seconds = totalSeconds % 60
  const parts = []
  if (hours) parts.push(`${hours}h`)
  if (minutes || hours) parts.push(`${minutes}m`)
  parts.push(`${seconds}s`)
  return { label: parts.join(" "), millis }
}

function buildLaunchCommand({ dataDir, identifier, serverPort, vitePort }) {
  return [
    "node scripts/v3-tauri-smoke-launch.mjs",
    "--run",
    "--force",
    "--data-dir",
    shellQuote(dataDir),
    "--vite-port",
    String(vitePort),
    "--server-port",
    String(serverPort),
    "--identifier",
    shellQuote(identifier),
  ].join(" ")
}

function runNode(argv) {
  return childProcess.execFileSync(process.execPath, argv, {
    cwd: process.cwd(),
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  })
}

function runGit(argv) {
  try {
    return childProcess.execFileSync("git", argv, {
      cwd: process.cwd(),
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    })
  } catch {
    return ""
  }
}

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"))
}

function parsePort(value, flag) {
  const port = Number.parseInt(value, 10)
  if (!Number.isInteger(port) || port < 1 || port > 65535) fail(`${flag} must be a TCP port.`)
  return port
}

function extractPort(url) {
  return new URL(url).port
}

function shellQuote(value) {
  const text = String(value)
  if (/^[A-Za-z0-9_./:=@+-]+$/.test(text)) return text
  return `'${text.replaceAll("'", "'\\''")}'`
}

function fail(message) {
  process.stderr.write(`${message}\n`)
  process.exit(1)
}
