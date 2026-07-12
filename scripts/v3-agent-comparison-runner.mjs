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

const system = parseSystem(args.system ?? "codex")
const repoRoot = process.cwd()
const plansDir = path.resolve(args.plansDir ?? process.env.HOPE_V3_PLANS_DIR ?? defaultPlansDir)
const artifactRel = args.artifact ?? "evidence/hope-claude-codex-comparison-2026-07-08.md"
const artifactPath = path.resolve(plansDir, artifactRel)
const runId = args.runId ?? `agent-comparison-${system}-${timestampForPath(new Date())}`
const baseDir = path.resolve(args.baseDir ?? path.join(os.tmpdir(), runId))
const runDir = path.resolve(args.runDir ?? path.join(baseDir, system))
const logsDir = path.join(runDir, "_comparison_logs")
const promptPath = path.join(logsDir, "prompt.txt")
const stdoutPath = path.join(logsDir, "stdout.txt")
const stderrPath = path.join(logsDir, "stderr.txt")
const metaPath = path.join(logsDir, "metadata.json")
const startedAt = new Date()
const timeoutSecs = Number(args.timeoutSecs ?? 3600)

fs.mkdirSync(logsDir, { recursive: true })
fs.writeFileSync(promptPath, sharedTaskPrompt())

const command = commandFor(system, runDir, promptPath, args)
const preflight = collectPreflight(command)
const packetBefore = renderPacket({
  artifactPath,
  command,
  exitCode: null,
  finishedAt: null,
  metaPath,
  notes: args.notes,
  promptPath,
  runDir,
  runId,
  startedAt: startedAt.toISOString(),
  stderrPath,
  stdoutPath,
  system,
  timeoutSecs,
})

if (args.append) appendArtifact(packetBefore)

let result = null
if (args.execute) {
  result = runCommand(command, runDir, timeoutSecs)
}
const finishedAt = new Date()

const metadata = {
  artifact: artifactPath,
  command,
  executed: Boolean(args.execute),
  exitCode: result?.status ?? null,
  finishedAt: finishedAt.toISOString(),
  preflight,
  promptPath,
  runDir,
  runId,
  startedAt: startedAt.toISOString(),
  stderrPath,
  stdoutPath,
  system,
  timeoutSecs,
  timedOut: Boolean(result?.timedOut),
}
fs.writeFileSync(metaPath, `${JSON.stringify(metadata, null, 2)}\n`)

if (result) {
  fs.writeFileSync(stdoutPath, result.stdout)
  fs.writeFileSync(stderrPath, result.stderr)
}

const packetAfter = renderPacket({
  artifactPath,
  command,
  exitCode: metadata.exitCode,
  finishedAt: metadata.finishedAt,
  metaPath,
  notes: args.notes,
  promptPath,
  runDir,
  runId,
  startedAt: metadata.startedAt,
  stderrPath,
  stdoutPath,
  system,
  timeoutSecs,
})

if (args.append && args.execute) appendArtifact(packetAfter)

if (args.json) {
  process.stdout.write(`${JSON.stringify(metadata, null, 2)}\n`)
} else {
  process.stdout.write(packetAfter)
  process.stdout.write("\n")
  if (args.append) process.stdout.write(`Appended runner packet to: ${artifactPath}\n`)
}

if (result?.status && !args.allowFailure) process.exit(result.status)

function parseArgs(argv) {
  const parsed = {
    allowFailure: false,
    append: false,
    artifact: null,
    baseDir: null,
    codexModel: null,
    execute: false,
    help: false,
    json: false,
    maxBudgetUsd: null,
    model: null,
    notes: null,
    plansDir: null,
    runDir: null,
    runId: null,
    system: null,
    timeoutSecs: null,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === "--allow-failure") parsed.allowFailure = true
    else if (arg === "--append") parsed.append = true
    else if (arg === "--artifact") parsed.artifact = argv[++i]
    else if (arg === "--base-dir") parsed.baseDir = argv[++i]
    else if (arg === "--execute") parsed.execute = true
    else if (arg === "--help" || arg === "-h") parsed.help = true
    else if (arg === "--json") parsed.json = true
    else if (arg === "--max-budget-usd") parsed.maxBudgetUsd = argv[++i]
    else if (arg === "--model") parsed.model = argv[++i]
    else if (arg === "--notes") parsed.notes = argv[++i]
    else if (arg === "--plans-dir") parsed.plansDir = argv[++i]
    else if (arg === "--run-dir") parsed.runDir = argv[++i]
    else if (arg === "--run-id") parsed.runId = argv[++i]
    else if (arg === "--system") parsed.system = argv[++i]
    else if (arg === "--timeout-secs") parsed.timeoutSecs = argv[++i]
    else fail(`Unknown argument: ${arg}`)
  }
  return parsed
}

function printHelp() {
  process.stdout.write(`Usage:
  node scripts/v3-agent-comparison-runner.mjs --system codex --execute --append [options]
  node scripts/v3-agent-comparison-runner.mjs --system claude-code --execute --append [options]
  node scripts/v3-agent-comparison-runner.mjs --system hope --append [options]

Purpose:
  Prepare or execute one raw run for the V3 Hope / Claude Code / Codex comparison.
  This runner saves the fixed shared prompt, command, stdout/stderr, and metadata.
  It does not inspect UI quality, score outputs, check coverage boxes, or mark strict proof passed.

Options:
  --system <name>            hope | claude-code | codex. Defaults to codex.
  --execute                  Actually run the command. Omit for dry-run packet.
  --append                   Append runner packets to the comparison artifact.
  --allow-failure            Return 0 even if the agent command exits non-zero.
  --json                     Print metadata JSON.
  --model <name>             Model label passed to the CLI where supported.
  --max-budget-usd <amount>  Claude Code print-mode budget cap.
  --timeout-secs <seconds>   Agent command timeout. Defaults to 3600.
  --base-dir <path>          Base directory for this comparison run.
  --run-dir <path>           Exact system run directory.
  --run-id <id>              Stable id for this run.
  --plans-dir <path>         Override Plans directory.
  --artifact <path>          Artifact path relative to Plans directory.
  --notes <text>             Reviewer notes.
  --help, -h                 Show this help.
`)
}

function commandFor(name, cwd, promptFile, input) {
  if (name === "codex") {
    const command = [
      "codex",
      "exec",
      "--cd",
      cwd,
      "--sandbox",
      "danger-full-access",
      "--dangerously-bypass-approvals-and-sandbox",
      "--skip-git-repo-check",
    ]
    if (input.model) command.push("--model", input.model)
    command.push(fs.readFileSync(promptFile, "utf8"))
    return command
  }
  if (name === "claude-code") {
    const command = [
      "claude",
      "--print",
      "--output-format",
      "text",
      "--permission-mode",
      "bypassPermissions",
    ]
    if (input.model) command.push("--model", input.model)
    if (input.maxBudgetUsd) command.push("--max-budget-usd", input.maxBudgetUsd)
    command.push(fs.readFileSync(promptFile, "utf8"))
    return command
  }
  return ["manual-hope-run", fs.readFileSync(promptFile, "utf8")]
}

function collectPreflight(command) {
  const binary = command[0]
  if (binary === "manual-hope-run") return { available: true, version: "manual Hope GUI run required" }
  const versionArgs = binary === "claude" ? ["--version"] : ["--version"]
  try {
    const version = childProcess.execFileSync(binary, versionArgs, {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    })
    return { available: true, version: version.trim() }
  } catch (error) {
    return { available: false, error: String(error?.message ?? error) }
  }
}

function runCommand(command, cwd, timeout) {
  const binary = command[0]
  if (binary === "manual-hope-run") {
    return {
      status: 0,
      stderr: "Hope product run is manual/GUI-owned. Use this packet to run the same prompt in Hope Goal mode.\n",
      stdout: "",
      timedOut: false,
    }
  }
  const result = childProcess.spawnSync(binary, command.slice(1), {
    cwd,
    encoding: "utf8",
    env: { ...process.env, NO_COLOR: "1" },
    maxBuffer: 20 * 1024 * 1024,
    timeout: timeout * 1000,
  })
  return {
    status: result.status ?? (result.signal ? 124 : 1),
    stderr: result.stderr ?? "",
    stdout: result.stdout ?? "",
    timedOut: result.error?.code === "ETIMEDOUT" || result.signal === "SIGTERM",
  }
}

function renderPacket(input) {
  return [
    `## Agent Comparison Raw Run Packet - ${new Date().toISOString()}`,
    "",
    "This packet records a raw comparison run setup or execution. It does not score quality, check strict-proof coverage, or mark the comparison passed.",
    "",
    "### Run",
    "",
    `- System: \`${displaySystem(input.system)}\``,
    `- Run id: \`${input.runId}\``,
    `- Run directory: \`${input.runDir}\``,
    `- Started at: \`${input.startedAt}\``,
    `- Finished at: \`${input.finishedAt ?? "pending"}\``,
    `- Timeout seconds: \`${input.timeoutSecs}\``,
    `- Exit code: \`${input.exitCode ?? "pending"}\``,
    `- Prompt file: \`${input.promptPath}\``,
    `- Stdout log: \`${input.stdoutPath}\``,
    `- Stderr log: \`${input.stderrPath}\``,
    `- Metadata: \`${input.metaPath}\``,
    `- Artifact: \`${input.artifactPath}\``,
    `- Notes: ${input.notes ?? "pending"}`,
    "",
    "Command:",
    "",
    "```bash",
    shellJoin(input.command),
    "```",
    "",
    "Next step after execution:",
    "",
    "1. Inspect the output directory and logs.",
    "2. Run the same validation checklist as the other systems.",
    "3. Append a `result` packet with `scripts/v3-agent-comparison-helper.mjs`.",
    "",
  ].join("\n")
}

function appendArtifact(packet) {
  fs.mkdirSync(path.dirname(artifactPath), { recursive: true })
  fs.appendFileSync(artifactPath, `\n${packet}\n`)
}

function parseSystem(value) {
  const normalized = value.toLowerCase()
  if (normalized === "hope") return "hope"
  if (normalized === "claude" || normalized === "claude-code" || normalized === "claude_code") return "claude-code"
  if (normalized === "codex") return "codex"
  fail("--system must be one of: hope, claude-code, codex")
}

function displaySystem(value) {
  if (value === "hope") return "Hope"
  if (value === "claude-code") return "Claude Code"
  return "Codex"
}

function timestampForPath(date) {
  return date.toISOString().replace(/[-:]/g, "").replace(/\..+/, "Z")
}

function shellJoin(argv) {
  return argv.map(shellQuote).join(" ")
}

function shellQuote(value) {
  const text = String(value)
  if (/^[A-Za-z0-9_./:=@+-]+$/.test(text)) return text
  return `'${text.replaceAll("'", "'\\''")}'`
}

function sharedTaskPrompt() {
  return `创建一个自包含的前端小项目「Mini Sprint Board」：它是一个给个人开发者用的轻量冲刺看板，可以管理本周要做的编码任务。请在当前工作区新建独立目录实现，不依赖后端，不需要联网；最终应能直接在浏览器打开或通过本地 dev server 运行。它应该像一个真正给个人开发者使用的小工具，而不是 demo 页面。

完成标准：
[required] 新建一个独立项目目录，包含可运行的前端应用和 README。
[required] UI 至少包含 3 列看板：Todo、In Progress、Done。
[required] 用户可以新增任务，任务字段至少包含标题、优先级、预计时间、标签。
[required] 用户可以编辑、删除任务，并在三列之间移动任务。
[required] 页面顶部显示统计信息：总任务数、已完成数、总预计时间、当前进行中任务数。
[required] 数据刷新后仍保留，使用 localStorage 或等价本地持久化。
[required] 提供一个干净的初始示例数据集，方便打开后直接体验。
[required] UI 要有基本产品质感：布局清晰、移动端可用、空状态友好、按钮/输入状态明确。
[required] 不引入不必要的复杂后端或外部服务。

验证要求：
- 说明如何运行。
- 运行可用的类型检查、构建或测试命令；如果项目技术栈没有这些命令，解释原因并至少做一次手动功能验收。
- 最终总结列出完成项、验证结果、未做项和风险。`
}

function fail(message) {
  process.stderr.write(`${message}\n`)
  process.exit(1)
}
