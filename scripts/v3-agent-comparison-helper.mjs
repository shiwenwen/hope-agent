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

const phase = args.phase ?? "prepare"
if (!["prepare", "result", "finish"].includes(phase)) {
  fail("--phase must be one of: prepare, result, finish")
}

const repoRoot = process.cwd()
const plansDir = path.resolve(args.plansDir ?? process.env.HOPE_V3_PLANS_DIR ?? defaultPlansDir)
const artifactRel = args.artifact ?? "evidence/hope-claude-codex-comparison-2026-07-08.md"
const artifactPath = path.resolve(plansDir, artifactRel)
const timestamp = new Date().toISOString()
const packageJson = readJson(path.join(repoRoot, "package.json"))
const context = {
  artifactPath,
  artifactRel,
  branch: runGit(["branch", "--show-current"]).trim() || "unknown",
  codexSummary: args.codexSummary,
  commit: runGit(["rev-parse", "--short=9", "HEAD"]).trim() || "unknown",
  completedRequired: args.completedRequired,
  finishDecision: args.finishDecision,
  finishedAt: args.finishedAt,
  goalId: args.goalId,
  hopeSummary: args.hopeSummary,
  loopId: args.loopId,
  manualSmoke: args.manualSmoke,
  model: args.model,
  notes: args.notes,
  nudges: args.nudges,
  outputArtifact: args.outputArtifact,
  packageVersion: packageJson.version ?? "unknown",
  permissionMode: args.permissionMode,
  phase,
  plansDir,
  recoveryNotes: args.recoveryNotes,
  reviewer: args.reviewer ?? "manual:<name>",
  repoRoot,
  runDir: args.runDir,
  screenshots: args.screenshots,
  sessionId: args.sessionId,
  startedAt: args.startedAt,
  system: args.system ? parseSystem(args.system) : null,
  timestamp,
  tokenCost: args.tokenCost,
  totalRequired: args.totalRequired ?? "9",
  transcriptId: args.transcriptId,
  validationResult: args.validationResult,
  workflowRunId: args.workflowRunId,
}

const packet = renderPacket(context)

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
        coverageHints: coverageHints(context),
        phase,
        plansDir,
        system: context.system,
        timestamp,
      },
      null,
      2,
    )}\n`,
  )
} else {
  process.stdout.write(packet)
  process.stdout.write("\n")
  if (args.append) process.stdout.write(`Appended comparison ${phase} packet to: ${artifactPath}\n`)
}

function parseArgs(argv) {
  const parsed = {
    append: false,
    artifact: null,
    codexSummary: null,
    completedRequired: null,
    finishDecision: null,
    finishedAt: null,
    goalId: null,
    help: false,
    hopeSummary: null,
    json: false,
    loopId: null,
    manualSmoke: null,
    model: null,
    notes: null,
    nudges: null,
    outputArtifact: null,
    permissionMode: null,
    phase: null,
    plansDir: null,
    recoveryNotes: null,
    reviewer: null,
    runDir: null,
    screenshots: null,
    sessionId: null,
    startedAt: null,
    system: null,
    tokenCost: null,
    totalRequired: null,
    transcriptId: null,
    validationResult: null,
    workflowRunId: null,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === "--append") parsed.append = true
    else if (arg === "--artifact") parsed.artifact = argv[++i]
    else if (arg === "--codex-summary") parsed.codexSummary = argv[++i]
    else if (arg === "--completed-required") parsed.completedRequired = argv[++i]
    else if (arg === "--finish-decision") parsed.finishDecision = argv[++i]
    else if (arg === "--finished-at") parsed.finishedAt = argv[++i]
    else if (arg === "--goal-id") parsed.goalId = argv[++i]
    else if (arg === "--help" || arg === "-h") parsed.help = true
    else if (arg === "--hope-summary") parsed.hopeSummary = argv[++i]
    else if (arg === "--json") parsed.json = true
    else if (arg === "--loop-id") parsed.loopId = argv[++i]
    else if (arg === "--manual-smoke") parsed.manualSmoke = argv[++i]
    else if (arg === "--model") parsed.model = argv[++i]
    else if (arg === "--notes") parsed.notes = argv[++i]
    else if (arg === "--nudges") parsed.nudges = argv[++i]
    else if (arg === "--output-artifact") parsed.outputArtifact = argv[++i]
    else if (arg === "--permission-mode") parsed.permissionMode = argv[++i]
    else if (arg === "--phase") parsed.phase = argv[++i]
    else if (arg === "--plans-dir") parsed.plansDir = argv[++i]
    else if (arg === "--recovery-notes") parsed.recoveryNotes = argv[++i]
    else if (arg === "--reviewer") parsed.reviewer = argv[++i]
    else if (arg === "--run-dir") parsed.runDir = argv[++i]
    else if (arg === "--screenshots") parsed.screenshots = argv[++i]
    else if (arg === "--session-id") parsed.sessionId = argv[++i]
    else if (arg === "--started-at") parsed.startedAt = argv[++i]
    else if (arg === "--system") parsed.system = argv[++i]
    else if (arg === "--token-cost") parsed.tokenCost = argv[++i]
    else if (arg === "--total-required") parsed.totalRequired = argv[++i]
    else if (arg === "--transcript-id") parsed.transcriptId = argv[++i]
    else if (arg === "--validation-result") parsed.validationResult = argv[++i]
    else if (arg === "--workflow-run-id") parsed.workflowRunId = argv[++i]
    else fail(`Unknown argument: ${arg}`)
  }
  return parsed
}

function printHelp() {
  process.stdout.write(`Usage:
  node scripts/v3-agent-comparison-helper.mjs --phase prepare [options]
  node scripts/v3-agent-comparison-helper.mjs --phase result --system <hope|claude-code|codex> --append [details]
  node scripts/v3-agent-comparison-helper.mjs --phase finish --append [details]

Purpose:
  Record execution packets for the V3 Hope / Claude Code / Codex comparison.
  This helper never runs agents, never scores from model claims alone, never
  checks coverage boxes, and never marks strict proof passed.

Options:
  --append                         Append packet to the comparison artifact.
  --json                           Print machine-readable metadata.
  --phase <name>                   prepare | result | finish.
  --plans-dir <path>               Override the V3 Plans directory.
  --artifact <path>                Artifact path relative to the Plans directory.
  --reviewer <name>                Reviewer label for packet/pass command.
  --system <name>                  hope | claude-code | codex for result packets.
  --run-dir <path>                 Clean worktree/output directory used by this system.
  --model <name>                   Model/version label.
  --permission-mode <mode>         Permission/sandbox mode used for the run.
  --started-at <iso>               Run start time.
  --finished-at <iso>              Run finish time.
  --completed-required <n>         Required criteria completed by reviewer.
  --total-required <n>             Required criteria total. Defaults to 9.
  --validation-result <text>       Validation commands and result.
  --manual-smoke <text>            Manual UI smoke result.
  --token-cost <text>              Token/cost/time data or strongest substitute.
  --nudges <text>                  User nudges after initial prompt.
  --recovery-notes <text>          Failure/recovery behavior.
  --output-artifact <path/url>     Final app/log/README path or URL.
  --screenshots <paths/urls>       Screenshots or screen recordings.
  --session-id <id>                Hope session id.
  --goal-id <id>                   Hope Goal id.
  --workflow-run-id <id>           Hope Workflow run id.
  --loop-id <id>                   Hope Loop id.
  --transcript-id <id>             Claude/Codex transcript, thread, or session id.
  --hope-summary <text>            Final Hope comparison summary for finish packet.
  --codex-summary <text>           Final Codex comparison summary for finish packet.
  --finish-decision <text>         Reviewer final comparison decision.
  --notes <text>                   Manual notes.
  --help, -h                       Show this help.
`)
}

function renderPacket(input) {
  if (input.phase === "prepare") return renderPreparePacket(input)
  if (input.phase === "result") return renderResultPacket(input)
  return renderFinishPacket(input)
}

function renderPreparePacket(input) {
  const stamp = input.timestamp.replace(/[-:]/g, "").replace(/\..+/, "")
  const baseDir = path.join(os.tmpdir(), `hope-agent-v3-agent-comparison-${stamp}`)
  return [
    `## Agent Comparison Prepare Packet - ${input.timestamp}`,
    "",
    "This packet prepares a fair Hope / Claude Code / Codex comparison. It does not execute agents, judge results, check boxes, or mark strict proof passed.",
    "",
    "### Environment",
    "",
    ...environmentLines(input),
    "",
    "### Clean Run Directories",
    "",
    "Use separate directories or worktrees so no system benefits from another system's output:",
    "",
    `- Hope: \`${path.join(baseDir, "hope")}\``,
    `- Claude Code: \`${path.join(baseDir, "claude-code")}\``,
    `- Codex: \`${path.join(baseDir, "codex")}\``,
    "",
    "```bash",
    `mkdir -p ${shellQuote(path.join(baseDir, "hope"))} ${shellQuote(path.join(baseDir, "claude-code"))} ${shellQuote(path.join(baseDir, "codex"))}`,
    "```",
    "",
    "### Fixed Shared Prompt",
    "",
    "```text",
    sharedTaskPrompt(),
    "```",
    "",
    "### Fair-run Rules",
    "",
    "- [ ] Use the exact same prompt and completion criteria for all three systems.",
    "- [ ] Use clean directories/worktrees and comparable permission/sandbox settings.",
    "- [ ] Do not add hidden hints to one system after seeing another system's result.",
    "- [ ] Record every material user nudge after the initial prompt.",
    "- [ ] Validate final artifacts with the same command/manual smoke checklist.",
    "- [ ] Record exact token/cost when exposed; otherwise record the strongest available substitute and why exact data is unavailable.",
    "",
    "### Result Packet Commands",
    "",
    "```bash",
    `node scripts/v3-agent-comparison-helper.mjs --phase result --append --reviewer ${shellQuote(input.reviewer)} --system hope --run-dir ${shellQuote(path.join(baseDir, "hope"))} --model "<model>" --permission-mode "<mode>" --completed-required <n> --validation-result "<commands/results>" --manual-smoke "<manual smoke>" --token-cost "<time/token/cost>" --nudges "<count/details>" --recovery-notes "<failures/recovery>"`,
    `node scripts/v3-agent-comparison-helper.mjs --phase result --append --reviewer ${shellQuote(input.reviewer)} --system claude-code --run-dir ${shellQuote(path.join(baseDir, "claude-code"))} --model "<model>" --permission-mode "<mode>" --completed-required <n> --validation-result "<commands/results>" --manual-smoke "<manual smoke>" --token-cost "<time/token/cost>" --nudges "<count/details>" --recovery-notes "<failures/recovery>"`,
    `node scripts/v3-agent-comparison-helper.mjs --phase result --append --reviewer ${shellQuote(input.reviewer)} --system codex --run-dir ${shellQuote(path.join(baseDir, "codex"))} --model "<model>" --permission-mode "<mode>" --completed-required <n> --validation-result "<commands/results>" --manual-smoke "<manual smoke>" --token-cost "<time/token/cost>" --nudges "<count/details>" --recovery-notes "<failures/recovery>"`,
    "```",
    "",
  ].join("\n")
}

function renderResultPacket(input) {
  if (!input.system) fail("--phase result requires --system <hope|claude-code|codex>")
  return [
    `## ${systemDisplayName(input.system)} Result Packet - ${input.timestamp}`,
    "",
    "This packet records one system's observed result. It must be backed by command output, artifact inspection, screenshots, transcripts, or reviewer notes.",
    "",
    "### Environment",
    "",
    ...environmentLines(input),
    "",
    "### Run Identity",
    "",
    `- System: \`${systemDisplayName(input.system)}\``,
    `- Model/version: \`${input.model ?? "pending"}\``,
    `- Permission/sandbox mode: \`${input.permissionMode ?? "pending"}\``,
    `- Run directory/worktree: \`${input.runDir ?? "pending"}\``,
    `- Started at: \`${input.startedAt ?? "pending"}\``,
    `- Finished at: \`${input.finishedAt ?? "pending"}\``,
    `- Session id: \`${input.sessionId ?? "pending"}\``,
    `- Goal id: \`${input.goalId ?? "pending"}\``,
    `- Workflow run id: \`${input.workflowRunId ?? "pending"}\``,
    `- Loop id: \`${input.loopId ?? "pending"}\``,
    `- Transcript/thread id: \`${input.transcriptId ?? "pending"}\``,
    "",
    "### Result Snapshot",
    "",
    `- Required criteria completed: \`${input.completedRequired ?? "pending"}/${input.totalRequired}\``,
    `- User nudges: ${input.nudges ?? "pending"}`,
    `- Validation result: ${input.validationResult ?? "pending"}`,
    `- Manual UI smoke: ${input.manualSmoke ?? "pending"}`,
    `- Time/token/cost: ${input.tokenCost ?? "pending"}`,
    `- Recovery notes: ${input.recoveryNotes ?? "pending"}`,
    `- Output artifact: ${input.outputArtifact ?? "pending"}`,
    `- Screenshots/logs: ${input.screenshots ?? "pending"}`,
    `- Notes: ${input.notes ?? "pending"}`,
    "",
    "### Same Validation Checklist",
    "",
    "- [ ] Independent project directory with README and runnable frontend app.",
    "- [ ] Three-column board: Todo, In Progress, Done.",
    "- [ ] Add task with title, priority, estimated time, and tags.",
    "- [ ] Edit, delete, and move tasks across columns.",
    "- [ ] Top statistics: total, completed, total estimate, in-progress count.",
    "- [ ] Local persistence survives reload.",
    "- [ ] Clean seed data is present.",
    "- [ ] Responsive product-quality UI with clear empty/input/button states.",
    "- [ ] No unnecessary backend or external service.",
    "",
    "Coverage hints from this packet:",
    "",
    ...Object.entries(coverageHints(input)).map(([key, value]) => `- ${key}: ${value ? "present" : "missing"}`),
    "",
  ].join("\n")
}

function renderFinishPacket(input) {
  const passCommand = [
    "node scripts/v3-strict-proof-record.mjs \\",
    "  --requirement codex_claude_comparison \\",
    "  --id codex_claude_comparison_2026_07_08 \\",
    "  --status passed \\",
    "  --evidence-kind real \\",
    `  --artifact ${input.artifactRel} \\`,
    `  --reviewer ${shellQuote(input.reviewer)} \\`,
    '  --summary "Hope / Claude Code / Codex real comparison completed." \\',
    "  --confirm-reviewed",
  ].join("\n")
  return [
    `## Agent Comparison Finish Packet - ${input.timestamp}`,
    "",
    "This packet summarizes reviewer judgment after all three real runs are recorded. It does not check boxes or mark strict proof passed.",
    "",
    "### Environment",
    "",
    ...environmentLines(input),
    "",
    "### Required Coverage Before Marking Passed",
    "",
    "- [ ] hope_result: Hope result packet includes artifact, validation, manual smoke, nudges, time/token/cost, and recovery notes.",
    "- [ ] claude_code_result: Claude Code result packet uses the same prompt and validation checklist.",
    "- [ ] codex_result: Codex result packet uses the same prompt and validation checklist.",
    "- [ ] validation_quality: all three outputs were validated with equivalent commands/manual smoke.",
    "- [ ] time_token_cost: all three runs have time/token/cost or documented substitutes.",
    "- [ ] recovery_notes: failures, interruptions, and recovery behavior are recorded for all three.",
    "",
    "### Reviewer Comparison",
    "",
    `- Hope summary: ${input.hopeSummary ?? "pending"}`,
    `- Codex/Claude comparison summary: ${input.codexSummary ?? "pending"}`,
    `- Final decision: ${input.finishDecision ?? "pending"}`,
    `- Notes: ${input.notes ?? "pending"}`,
    "",
    "Mark passed only after the artifact's Required Coverage and Reviewer Decision boxes are checked:",
    "",
    "```bash",
    passCommand,
    "```",
    "",
  ].join("\n")
}

function environmentLines(input) {
  return [
    `- Hope commit: \`${input.commit}\``,
    `- Branch: \`${input.branch}\``,
    `- App build/version: \`${input.packageVersion}\``,
    `- Workspace/worktree: \`${input.repoRoot}\``,
    `- Plans dir: \`${input.plansDir}\``,
    `- Artifact: \`${input.artifactPath}\``,
    `- Reviewer: \`${input.reviewer}\``,
  ]
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

function parseSystem(value) {
  const normalized = value.toLowerCase()
  if (normalized === "hope") return "hope"
  if (normalized === "claude" || normalized === "claude-code" || normalized === "claude_code") return "claude-code"
  if (normalized === "codex") return "codex"
  fail("--system must be one of: hope, claude-code, codex")
}

function systemDisplayName(system) {
  if (system === "hope") return "Hope"
  if (system === "claude-code") return "Claude Code"
  if (system === "codex") return "Codex"
  return "Unknown"
}

function coverageHints(input) {
  return {
    hope_result: input.system === "hope" && Boolean(input.completedRequired || input.outputArtifact),
    claude_code_result: input.system === "claude-code" && Boolean(input.completedRequired || input.outputArtifact),
    codex_result: input.system === "codex" && Boolean(input.completedRequired || input.outputArtifact),
    validation_quality: Boolean(input.validationResult || input.manualSmoke),
    time_token_cost: Boolean(input.tokenCost || (input.startedAt && input.finishedAt)),
    recovery_notes: Boolean(input.recoveryNotes),
  }
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

function shellQuote(value) {
  const text = String(value)
  if (/^[A-Za-z0-9_./:=@+-]+$/.test(text)) return text
  return `'${text.replaceAll("'", "'\\''")}'`
}

function fail(message) {
  process.stderr.write(`${message}\n`)
  process.exit(1)
}
