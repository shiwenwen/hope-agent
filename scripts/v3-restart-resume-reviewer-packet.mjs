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

const plansDir = path.resolve(args.plansDir ?? process.env.HOPE_V3_PLANS_DIR ?? defaultPlansDir)
const artifactRel = args.artifact ?? "evidence/restart-resume-matrix-2026-07-08.md"
const artifactPath = path.resolve(plansDir, artifactRel)
const content = fs.existsSync(artifactPath) ? fs.readFileSync(artifactPath, "utf8") : ""
const timestamp = new Date().toISOString()

if (!content) {
  fail(`Restart/resume artifact not found or empty: ${artifactPath}`)
}

const signals = buildSignalMap(content)
const coverageRows = buildCoverageRows(signals)
const packet = renderPacket({
  artifactPath,
  artifactRel,
  branch: runGit(["branch", "--show-current"]).trim() || "unknown",
  commit: runGit(["rev-parse", "--short=9", "HEAD"]).trim() || "unknown",
  coverageRows,
  plansDir,
  reviewer: args.reviewer ?? "manual:<name>",
  repoRoot: process.cwd(),
  signals,
  timestamp,
})

if (args.append) {
  fs.appendFileSync(artifactPath, `\n${packet}\n`)
}

if (args.json) {
  process.stdout.write(
    `${JSON.stringify(
      {
        artifactPath,
        appended: args.append,
        coverageRows,
        plansDir,
        signals,
      },
      null,
      2,
    )}\n`,
  )
} else {
  process.stdout.write(packet)
  process.stdout.write("\n")
  if (args.append) process.stdout.write(`Appended reviewer packet to: ${artifactPath}\n`)
}

function parseArgs(argv) {
  const parsed = {
    append: false,
    artifact: null,
    help: false,
    json: false,
    plansDir: null,
    reviewer: null,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === "--append") parsed.append = true
    else if (arg === "--artifact") parsed.artifact = argv[++i]
    else if (arg === "--help" || arg === "-h") parsed.help = true
    else if (arg === "--json") parsed.json = true
    else if (arg === "--plans-dir") parsed.plansDir = argv[++i]
    else if (arg === "--reviewer") parsed.reviewer = argv[++i]
    else fail(`Unknown argument: ${arg}`)
  }
  return parsed
}

function printHelp() {
  process.stdout.write(`Usage:
  node scripts/v3-restart-resume-reviewer-packet.mjs [options]
  node scripts/v3-restart-resume-reviewer-packet.mjs --append --reviewer "<name>"

Purpose:
  Summarize the current real restart/resume evidence into a reviewer packet.
  The packet maps each strict coverage item to existing samples and highlights
  the remaining human decision. It never checks coverage boxes or marks strict
  proof passed.

Options:
  --append              Append the packet to the restart/resume evidence artifact.
  --json                Print machine-readable coverage mapping.
  --plans-dir <path>    Override the V3 Plans directory.
  --artifact <path>     Artifact path relative to the Plans directory.
  --reviewer <name>     Reviewer label to include in the packet.
  --help, -h            Show this help.
`)
}

function buildSignalMap(content) {
  const has = (needle) => content.includes(needle)
  return {
    activeGoalBasic: has("goal_7def4e05c0304a498bbc2d62914cce12") && has("Goal pause/resume"),
    approvalWaiting:
      has("approval-waiting") && has("awaiting_approval") && has("no auto-approval or background execution"),
    backgroundInterrupted:
      has("job_9a29f08d467541e79794cced8a8b7046") && has("interrupted by application restart"),
    dynamicLoopRestart:
      has("dynamic-loop-restart-20260708") &&
      has("lrun_5b9da20c268240b199ba6089c53e7fc8") &&
      has("dynamic_fallback_120s"),
    guiConfirmationMissing:
      has("still needs GUI/user-visible confirmation") || has("needs GUI/user-visible confirmation"),
    incognitoGuard:
      has("incognito-guard-valid-workflow-20260708") &&
      has("Cannot create durable goal") &&
      has("Cannot create durable loop schedule") &&
      has("Cannot create durable workflow run"),
    longRestart:
      has("long-restart-20260708") &&
      has("run_recovery_claimed") &&
      has("guarded_repair_validation_failed"),
    successfulTransparentResumeMissing:
      has("not evidence of a successful resumed long command") || has("does not prove transparent job continuation"),
  }
}

function buildCoverageRows(signals) {
  return [
    {
      coverage: "active_goal_restart",
      evidence: signals.activeGoalBasic
        ? "basic owner-API kill/restart read-back, same Goal id, Goal pause/resume after restart."
        : "Missing active Goal restart sample.",
      currentDecision: signals.activeGoalBasic ? "candidate-partial" : "missing",
      remaining: signals.activeGoalBasic
        ? "Needs GUI/user-visible confirmation before checking coverage."
        : "Run basic restart setup/verify sample.",
    },
    {
      coverage: "running_workflow_restart",
      evidence: signals.longRestart
        ? "long-restart-20260708 observed running Workflow, killed process group, recovery claimed, interrupted validation visible, Goal blocked."
        : "Missing running Workflow restart sample.",
      currentDecision: signals.longRestart ? "reviewer-choice" : "missing",
      remaining: signals.longRestart
        ? "Under the V3 acceptance policy, reviewer may accept conservative interruption visibility + Goal blocked if user-visible recovery is sufficient."
        : "Run long-workflow matrix sample during running window.",
    },
    {
      coverage: "dynamic_loop_restart",
      evidence: signals.dynamicLoopRestart
        ? "dynamic-loop-restart-20260708 persisted the same Loop run across restart and recorded fallback/no-progress metadata."
        : "Missing dynamic Loop restart sample.",
      currentDecision: signals.dynamicLoopRestart ? "candidate-partial" : "missing",
      remaining: signals.dynamicLoopRestart
        ? "Needs GUI/user-visible confirmation before checking coverage."
        : "Run dynamic-loop matrix sample.",
    },
    {
      coverage: "approval_waiting_restart",
      evidence: signals.approvalWaiting
        ? "approval-waiting sample stayed awaiting_approval after kill/restart with no auto-approval or background execution."
        : "Missing approval waiting restart sample.",
      currentDecision: signals.approvalWaiting ? "candidate-partial" : "missing",
      remaining: signals.approvalWaiting
        ? "Needs GUI/user-visible confirmation before checking coverage."
        : "Run approval-waiting setup/verify sample.",
    },
    {
      coverage: "background_job_waiting_restart",
      evidence: signals.backgroundInterrupted
        ? "long-restart-20260708 observed a running background job before kill and an interrupted status after restart."
        : "Missing background job waiting restart sample.",
      currentDecision: signals.backgroundInterrupted ? "reviewer-choice" : "missing",
      remaining: signals.backgroundInterrupted
        ? "Under the V3 acceptance policy, durable interruption visibility is acceptable if no duplicate action occurs and Goal/Workflow recovery is clear."
        : "Run a long-workflow/background job sample and kill during running window.",
    },
    {
      coverage: "incognito_control",
      evidence: signals.incognitoGuard
        ? "incognito-guard-valid-workflow-20260708 rejects durable Goal, Loop, and Script-Gate-valid Workflow creation."
        : "Missing incognito durable-control fail-closed sample.",
      currentDecision: signals.incognitoGuard ? "supporting-control-present" : "missing",
      remaining: signals.incognitoGuard
        ? "Supporting control evidence present; not a manifest coverage label but required by expected result."
        : "Run incognito-guard matrix sample.",
    },
  ]
}

function renderPacket({ artifactPath, artifactRel, branch, commit, coverageRows, plansDir, reviewer, repoRoot, signals, timestamp }) {
  const passCommand = [
    "node scripts/v3-strict-proof-record.mjs \\",
    "  --requirement real_restart_resume_matrix \\",
    "  --id real_restart_resume_matrix_2026_07_08 \\",
    "  --status passed \\",
    `  --artifact ${artifactRel} \\`,
    `  --reviewer ${shellQuote(reviewer)} \\`,
    '  --summary "Real restart/resume matrix proof completed." \\',
    "  --confirm-reviewed",
  ].join("\n")

  const strictExtraCommand = [
    "node scripts/v3-restart-resume-matrix-runner.mjs \\",
    "  --scenario long-workflow \\",
    "  --long-sleep-secs 180 \\",
    "  --wait-state-secs 25 \\",
    "  --wait-health-secs 90 \\",
    "  --kill-delay-secs 1 \\",
    "  --restart-delay-secs 3 \\",
    "  --run-id long-restart-extra-YYYYMMDD \\",
    "  --json",
  ].join("\n")

  return [
    `## Restart / Resume Reviewer Packet - ${timestamp}`,
    "",
    "This packet summarizes existing restart/resume evidence for reviewer judgment. It does not prove coverage by itself, does not check boxes, and does not mark strict proof passed.",
    "",
    "### Environment",
    "",
    `- Hope commit: \`${commit}\``,
    `- Branch: \`${branch}\``,
    `- Workspace/worktree: \`${repoRoot}\``,
    `- Plans dir: \`${plansDir}\``,
    `- Artifact: \`${artifactPath}\``,
    `- Reviewer: \`${reviewer}\``,
    "",
    "### Evidence Signals",
    "",
    ...Object.entries(signals).map(([key, value]) => `- ${key}: ${value ? "yes" : "no"}`),
    "",
    "### Coverage Map",
    "",
    "| Coverage | Existing evidence | Current decision | Remaining reviewer action |",
    "| --- | --- | --- | --- |",
    ...coverageRows.map(
      (row) =>
        `| \`${row.coverage}\` | ${escapeTable(row.evidence)} | \`${row.currentDecision}\` | ${escapeTable(row.remaining)} |`,
    ),
    "",
    "### Reviewer Decision Point",
    "",
    "- V3 acceptance policy is **durable conservative recovery**: no silent loss, no duplicate external action, interrupted background work is visible, and Goal/Workflow becomes blocked or user-actionable.",
    "- Current `long-restart-20260708` can be accepted for `running_workflow_restart` and `background_job_waiting_restart` after reviewer confirms the recovery is sufficiently user-visible.",
    "- Transparent command continuation or automatic successful retry is a future enhancement, not a V3 closure requirement.",
    "- In either route, do not mark this blocker passed until Required Coverage and Reviewer Decision checkboxes are all manually checked in the artifact.",
    "",
    "### Optional Stricter Extra Sample",
    "",
    "```bash",
    strictExtraCommand,
    "```",
    "",
    "### Mark Passed After Manual Review",
    "",
    "```bash",
    passCommand,
    "```",
    "",
  ].join("\n")
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

function escapeTable(value) {
  return String(value).replaceAll("|", "\\|").replace(/\n+/g, " ")
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
