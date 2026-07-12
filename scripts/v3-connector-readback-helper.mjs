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
if (!["prepare", "approval", "execution", "readback", "rollback", "finish"].includes(phase)) {
  fail("--phase must be one of: prepare, approval, execution, readback, rollback, finish")
}

const repoRoot = process.cwd()
const plansDir = path.resolve(args.plansDir ?? process.env.HOPE_V3_PLANS_DIR ?? defaultPlansDir)
const artifactRel = args.artifact ?? "evidence/connector-readback-2026-07-08.md"
const artifactPath = path.resolve(plansDir, artifactRel)
const timestamp = new Date().toISOString()
const packageJson = readJson(path.join(repoRoot, "package.json"))
const context = {
  action: args.action,
  approvalId: args.approvalId,
  artifactPath,
  artifactRel,
  branch: runGit(["branch", "--show-current"]).trim() || "unknown",
  commit: runGit(["rev-parse", "--short=9", "HEAD"]).trim() || "unknown",
  connector: args.connector,
  account: args.account,
  evidenceKind: args.evidenceKind ?? "sandbox",
  executionResult: args.executionResult,
  expectedState: args.expectedState,
  goalId: args.goalId,
  notes: args.notes,
  objectId: args.objectId,
  objectUrl: args.objectUrl,
  packageVersion: packageJson.version ?? "unknown",
  phase,
  plansDir,
  readbackResult: args.readbackResult,
  reviewer: args.reviewer ?? "manual:<name>",
  rollbackResult: args.rollbackResult,
  repoRoot,
  sessionId: args.sessionId,
  timestamp,
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
        evidenceKind: context.evidenceKind,
        phase,
        plansDir,
        timestamp,
      },
      null,
      2,
    )}\n`,
  )
} else {
  process.stdout.write(packet)
  process.stdout.write("\n")
  if (args.append) process.stdout.write(`Appended connector ${phase} packet to: ${artifactPath}\n`)
}

function parseArgs(argv) {
  const parsed = {
    account: null,
    action: null,
    append: false,
    approvalId: null,
    artifact: null,
    connector: null,
    evidenceKind: null,
    executionResult: null,
    expectedState: null,
    goalId: null,
    help: false,
    json: false,
    notes: null,
    objectId: null,
    objectUrl: null,
    phase: null,
    plansDir: null,
    readbackResult: null,
    reviewer: null,
    rollbackResult: null,
    sessionId: null,
    workflowRunId: null,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === "--account") parsed.account = argv[++i]
    else if (arg === "--action") parsed.action = argv[++i]
    else if (arg === "--append") parsed.append = true
    else if (arg === "--approval-id") parsed.approvalId = argv[++i]
    else if (arg === "--artifact") parsed.artifact = argv[++i]
    else if (arg === "--connector") parsed.connector = argv[++i]
    else if (arg === "--evidence-kind") parsed.evidenceKind = parseEvidenceKind(argv[++i])
    else if (arg === "--execution-result") parsed.executionResult = argv[++i]
    else if (arg === "--expected-state") parsed.expectedState = argv[++i]
    else if (arg === "--goal-id") parsed.goalId = argv[++i]
    else if (arg === "--help" || arg === "-h") parsed.help = true
    else if (arg === "--json") parsed.json = true
    else if (arg === "--notes") parsed.notes = argv[++i]
    else if (arg === "--object-id") parsed.objectId = argv[++i]
    else if (arg === "--object-url") parsed.objectUrl = argv[++i]
    else if (arg === "--phase") parsed.phase = argv[++i]
    else if (arg === "--plans-dir") parsed.plansDir = argv[++i]
    else if (arg === "--readback-result") parsed.readbackResult = argv[++i]
    else if (arg === "--reviewer") parsed.reviewer = argv[++i]
    else if (arg === "--rollback-result") parsed.rollbackResult = argv[++i]
    else if (arg === "--session-id") parsed.sessionId = argv[++i]
    else if (arg === "--workflow-run-id") parsed.workflowRunId = argv[++i]
    else fail(`Unknown argument: ${arg}`)
  }
  return parsed
}

function printHelp() {
  process.stdout.write(`Usage:
  node scripts/v3-connector-readback-helper.mjs --phase prepare [options]
  node scripts/v3-connector-readback-helper.mjs --phase approval --append [details]
  node scripts/v3-connector-readback-helper.mjs --phase execution --append [details]
  node scripts/v3-connector-readback-helper.mjs --phase readback --append [details]
  node scripts/v3-connector-readback-helper.mjs --phase rollback --append [details]
  node scripts/v3-connector-readback-helper.mjs --phase finish --append [details]

Purpose:
  Record staged evidence packets for V3 connector execution + read-back proof.
  This helper never calls a connector, never mutates an external system, never
  checks coverage boxes, and never marks strict proof passed.

Options:
  --append                         Append packet to the connector read-back artifact.
  --json                           Print machine-readable metadata.
  --phase <name>                   prepare | approval | execution | readback | rollback | finish.
  --plans-dir <path>               Override the V3 Plans directory.
  --artifact <path>                Artifact path relative to the Plans directory.
  --reviewer <name>                Reviewer label for packet/pass command.
  --evidence-kind real|sandbox     Defaults to sandbox.
  --connector <name>               Connector name, e.g. google-drive, gmail, github.
  --account <label>                Test account or sandbox account label.
  --action <text>                  Intended connector action.
  --object-id <id>                 External/sandbox object id.
  --object-url <url>               External/sandbox object URL.
  --expected-state <text>          Expected post-action external state.
  --approval-id <id>               Approval or sandbox evidence id.
  --execution-result <text>        Observed connector execution result.
  --readback-result <text>         Observed connector post-action read-back result.
  --rollback-result <text>         Observed rollback/recovery/cleanup result.
  --session-id <id>                Hope session id.
  --goal-id <id>                   Goal id.
  --workflow-run-id <id>           Workflow run id.
  --notes <text>                   Manual notes.
  --help, -h                       Show this help.
`)
}

function renderPacket(input) {
  const title = {
    approval: "Approval / Sandbox Evidence",
    execution: "Connector Execution Evidence",
    finish: "Connector Read-back Finish Packet",
    prepare: "Connector Read-back Prepare Packet",
    readback: "Post-action Read-back Evidence",
    rollback: "Rollback / Recovery Evidence",
  }[input.phase]

  return [
    `## ${title} - ${input.timestamp}`,
    "",
    "This packet records connector E2E evidence context. It does not call connectors, check boxes, or mark strict proof passed.",
    "",
    "### Environment",
    "",
    `- Hope commit: \`${input.commit}\``,
    `- Branch: \`${input.branch}\``,
    `- App build/version: \`${input.packageVersion}\``,
    `- Workspace/worktree: \`${input.repoRoot}\``,
    `- Plans dir: \`${input.plansDir}\``,
    `- Artifact: \`${input.artifactPath}\``,
    `- Reviewer: \`${input.reviewer}\``,
    `- Evidence kind: \`${input.evidenceKind}\``,
    "",
    ...connectorLines(input),
    "",
    ...phaseBody(input),
    "",
  ].join("\n")
}

function connectorLines(input) {
  return [
    "### Connector Scope",
    "",
    `- Connector: \`${input.connector ?? "pending"}\``,
    `- Test/sandbox account: \`${input.account ?? "pending"}\``,
    `- Action: ${input.action ?? "pending"}`,
    `- External object id: \`${input.objectId ?? "pending"}\``,
    `- External object URL: ${input.objectUrl ?? "pending"}`,
    `- Expected post-action state: ${input.expectedState ?? "pending"}`,
    `- Approval/sandbox evidence id: \`${input.approvalId ?? "pending"}\``,
    `- Session id: \`${input.sessionId ?? "pending"}\``,
    `- Goal id: \`${input.goalId ?? "pending"}\``,
    `- Workflow run id: \`${input.workflowRunId ?? "pending"}\``,
    `- Notes: ${input.notes ?? "pending"}`,
  ]
}

function phaseBody(input) {
  if (input.phase === "prepare") return prepareBody(input)
  if (input.phase === "approval") return approvalBody(input)
  if (input.phase === "execution") return executionBody(input)
  if (input.phase === "readback") return readbackBody(input)
  if (input.phase === "rollback") return rollbackBody(input)
  return finishBody(input)
}

function prepareBody(input) {
  return [
    "### Recommended Safe Smoke",
    "",
    "Use a disposable sandbox/test object. A good default is a Google Drive/Docs/Sheets test artifact named `Hope V3 Connector Readback Smoke <timestamp>` in a test folder/account.",
    "",
    "Required sequence:",
    "",
    "1. Create or select a sandbox/test object.",
    "2. Record approval or sandbox isolation before mutation.",
    "3. Execute one harmless connector mutation.",
    "4. Read the same object back through the connector after execution.",
    "5. Compare read-back state with expected state.",
    "6. Delete/trash/archive/restore the object or record why cleanup is safe to skip.",
    "",
    "Suggested Goal prompt:",
    "",
    "```text",
    connectorPrompt(input),
    "```",
    "",
    "Use these staged commands as evidence is observed:",
    "",
    "```bash",
    `node scripts/v3-connector-readback-helper.mjs --phase approval --append --reviewer ${shellQuote(input.reviewer)} --connector <name> --account <test-account> --action "<action>" --expected-state "<state>" --approval-id <approval-or-sandbox-id>`,
    `node scripts/v3-connector-readback-helper.mjs --phase execution --append --reviewer ${shellQuote(input.reviewer)} --connector <name> --object-id <id> --execution-result "<actual execution result>"`,
    `node scripts/v3-connector-readback-helper.mjs --phase readback --append --reviewer ${shellQuote(input.reviewer)} --connector <name> --object-id <id> --readback-result "<connector read-back result>"`,
    `node scripts/v3-connector-readback-helper.mjs --phase rollback --append --reviewer ${shellQuote(input.reviewer)} --connector <name> --object-id <id> --rollback-result "<cleanup/recovery result>"`,
    `node scripts/v3-connector-readback-helper.mjs --phase finish --append --reviewer ${shellQuote(input.reviewer)} --connector <name> --object-id <id> --approval-id <approval-id> --execution-result "<result>" --readback-result "<read-back>" --rollback-result "<cleanup>"`,
    "```",
  ]
}

function approvalBody(input) {
  return [
    "### Approval / Sandbox Checklist",
    "",
    "- [ ] User approval prompt or sandbox isolation is visible before mutation.",
    "- [ ] Test account/folder/object is clearly non-production.",
    "- [ ] Exact intended action and expected read-back state are recorded.",
    "- [ ] Incognito is not used for durable evidence.",
    "",
    `Approval/sandbox evidence: ${input.approvalId ?? "pending"}`,
  ]
}

function executionBody(input) {
  return [
    "### Execution Checklist",
    "",
    "- [ ] Connector mutation actually executed against the test/sandbox object.",
    "- [ ] Execution result includes connector/action/result id or status.",
    "- [ ] Execution was not inferred only from local logs or model text.",
    "",
    `Execution result: ${input.executionResult ?? "pending"}`,
  ]
}

function readbackBody(input) {
  return [
    "### Read-back Checklist",
    "",
    "- [ ] Same connector read back the target object after execution.",
    "- [ ] Read-back state matches the expected post-action state.",
    "- [ ] Read-back is not copied from model summary, local cache, or fixture-only data.",
    "",
    `Expected state: ${input.expectedState ?? "pending"}`,
    `Read-back result: ${input.readbackResult ?? "pending"}`,
  ]
}

function rollbackBody(input) {
  return [
    "### Rollback / Recovery Checklist",
    "",
    "- [ ] Cleanup, rollback, archive/trash, or recovery plan is recorded.",
    "- [ ] If cleanup changed external state, cleanup was read back or otherwise verified.",
    "- [ ] If no cleanup was needed, the reason is explicit and safe.",
    "",
    `Rollback/recovery result: ${input.rollbackResult ?? "pending"}`,
  ]
}

function finishBody(input) {
  const passCommand = [
    "node scripts/v3-strict-proof-record.mjs \\",
    "  --requirement connector_readback \\",
    "  --id connector_readback_2026_07_08 \\",
    "  --status passed \\",
    "  --evidence-kind sandbox \\",
    `  --artifact ${input.artifactRel} \\`,
    `  --reviewer ${shellQuote(input.reviewer)} \\`,
    '  --summary "Connector execution plus post-action read-back proof completed." \\',
    "  --confirm-reviewed",
  ].join("\n")
  return [
    "### Finish Checklist",
    "",
    "- [ ] connector_execution: real or sandbox connector mutation executed.",
    "- [ ] post_action_readback: post-action state was read back from the connector.",
    "- [ ] approval_or_sandbox: explicit approval or sandbox/test-account evidence exists.",
    "- [ ] rollback_or_recovery: cleanup, rollback, archive/trash, or safe recovery explanation exists.",
    "- [ ] No permission, approval, connector guard, or incognito behavior was bypassed.",
    "",
    "Coverage hints:",
    "",
    ...Object.entries(coverageHints(input)).map(([key, value]) => `- ${key}: ${value ? "present" : "missing"}`),
    "",
    "Mark passed only after the artifact's Required Coverage and Reviewer Decision boxes are checked:",
    "",
    "```bash",
    passCommand,
    "```",
  ]
}

function connectorPrompt(input) {
  const connector = input.connector ?? "<connector>"
  const action = input.action ?? "<harmless test mutation>"
  const expected = input.expectedState ?? "<expected post-action state>"
  return `请在测试账号或沙箱对象中完成一个低风险连接器端到端验收，不要修改真实业务对象。

连接器：${connector}
动作：${action}
期望读回状态：${expected}

完成标准：
[required] 先展示目标对象、动作内容、风险和回滚/清理方案。
[required] 在执行外部修改前获得显式批准，或证明这是 sandbox/test account。
[required] 执行连接器动作后，必须通过同一连接器读取目标对象当前状态。
[required] 对比读回状态和期望状态，记录 connector/action/result/object id。
[required] 完成清理、回滚、归档，或说明为什么该测试对象可安全保留。
[required] 最终总结说明审批/沙箱证据、执行结果、读回结果、回滚/恢复结果。`
}

function coverageHints(input) {
  return {
    approval_or_sandbox: Boolean(input.approvalId || input.evidenceKind === "sandbox"),
    connector_execution: Boolean(input.executionResult),
    post_action_readback: Boolean(input.readbackResult),
    rollback_or_recovery: Boolean(input.rollbackResult),
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

function parseEvidenceKind(value) {
  if (value === "real" || value === "sandbox") return value
  fail("--evidence-kind must be real or sandbox")
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
