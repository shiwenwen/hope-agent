#!/usr/bin/env node
import fs from "node:fs"
import os from "node:os"
import path from "node:path"
import {
  reviewerDecisionLabels,
  strictProofClosureOrder,
  strictProofRequirements,
} from "./v3-strict-proof-requirements.mjs"

const defaultPlansDir = path.join(
  os.homedir(),
  "Library/Mobile Documents/com~apple~CloudDocs/HopeAI/Hope Agent/Plans/hope-agent-control-plane-plans-2026-07-05/11-agent-control-plane-v3-claude-parity",
)

const requirements = strictProofRequirements
const closureOrder = strictProofClosureOrder

const args = parseArgs(process.argv.slice(2))
const plansDir = path.resolve(args.plansDir ?? process.env.HOPE_V3_PLANS_DIR ?? defaultPlansDir)
const manifestPath = path.join(plansDir, "v3-strict-proof-evidence.json")

if (args.help) {
  printHelp()
  process.exit(0)
}

if (args.json && !args.list && !args.checkReady && !args.next) {
  fail("--json is only supported with --list, --next, or --check-ready.")
}

if (args.checkReady) {
  checkReady(readManifest(manifestPath), { json: args.json })
  process.exit(0)
}

if (args.next) {
  printNext(readManifest(manifestPath), { json: args.json, skip: args.skip })
  process.exit(0)
}

if (args.list) {
  printStatusList(readManifest(manifestPath), { json: args.json })
  process.exit(0)
}

const spec = requirements[args.requirement]
if (!spec) {
  fail(`Unknown or missing --requirement. Expected one of: ${Object.keys(requirements).join(", ")}`)
}

const today = new Date().toISOString().slice(0, 10)
const id = args.id ?? `${args.requirement}_${today.replaceAll("-", "_")}`
const status = args.status ?? "pending"
const evidenceKind = args.evidenceKind ?? spec.defaultEvidenceKind
const artifactRel = normalizeArtifactPath(args.artifact ?? `evidence/${spec.artifactSlug}-${today}.md`)
const artifactPath = path.resolve(plansDir, artifactRel)
const reviewer = args.reviewer ?? "manual:<name>"
const summary = args.summary ?? spec.prompt
const performedAt = args.performedAt ?? new Date().toISOString()

if (!["pending", "passed"].includes(status)) fail('--status must be "pending" or "passed".')
if (!spec.allowedEvidenceKinds.includes(evidenceKind)) {
  fail(`--evidence-kind for ${args.requirement} must be one of: ${spec.allowedEvidenceKinds.join(", ")}`)
}
if (status === "passed") {
  if (!args.confirmReviewed) fail("Refusing to mark strict evidence passed without --confirm-reviewed.")
  if (!args.reviewer || reviewer === "manual:<name>") fail("Passed evidence requires --reviewer.")
  if (!args.summary) fail("Passed evidence requires --summary.")
  if (!fs.existsSync(artifactPath)) fail(`Passed evidence artifact must already exist: ${artifactRel}`)
  const artifactErrors = validateStrictArtifactContent(artifactPath, spec.coverage, args.requirement)
  if (artifactErrors.length > 0) {
    fail(`Passed evidence artifact is not ready:\n${artifactErrors.map((error) => `- ${error}`).join("\n")}`)
  }
}

fs.mkdirSync(plansDir, { recursive: true })
fs.mkdirSync(path.dirname(artifactPath), { recursive: true })

if (!fs.existsSync(artifactPath)) {
  fs.writeFileSync(artifactPath, renderArtifactTemplate({ id, status, spec, artifactRel, reviewer, summary, performedAt }))
}

const manifest = readManifest(manifestPath)
const entry = {
  id,
  requirement: args.requirement,
  status,
  evidenceKind,
  performedAt,
  reviewer,
  summary,
  artifacts: [artifactRel],
  coverage: spec.coverage,
}

const existingIndex = manifest.evidence.findIndex((item) => item && item.id === id)
if (existingIndex >= 0) manifest.evidence[existingIndex] = entry
else manifest.evidence.push(entry)

manifest.updatedAt = new Date().toISOString()
writeJson(manifestPath, manifest)

process.stdout.write(
  [
    `Recorded ${status} strict proof entry: ${id}`,
    `Manifest: ${manifestPath}`,
    `Artifact: ${artifactPath}`,
    status === "pending"
      ? "Next: fill the artifact with real observations, then rerun this command with --status passed --confirm-reviewed."
      : "Next: run scripts/v3-strict-proof-audit.mjs and confirm this requirement passes.",
    "",
  ].join("\n"),
)

function parseArgs(argv) {
  const parsed = {
    artifact: null,
    checkReady: false,
    confirmReviewed: false,
    evidenceKind: null,
    help: false,
    id: null,
    json: false,
    list: false,
    next: false,
    performedAt: null,
    plansDir: null,
    requirement: null,
    reviewer: null,
    skip: [],
    status: null,
    summary: null,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === "--artifact") parsed.artifact = argv[++i]
    else if (arg === "--check-ready") parsed.checkReady = true
    else if (arg === "--confirm-reviewed") parsed.confirmReviewed = true
    else if (arg === "--evidence-kind") parsed.evidenceKind = argv[++i]
    else if (arg === "--help" || arg === "-h") parsed.help = true
    else if (arg === "--id") parsed.id = argv[++i]
    else if (arg === "--json") parsed.json = true
    else if (arg === "--list") parsed.list = true
    else if (arg === "--next") parsed.next = true
    else if (arg === "--performed-at") parsed.performedAt = argv[++i]
    else if (arg === "--plans-dir") parsed.plansDir = argv[++i]
    else if (arg === "--requirement") parsed.requirement = argv[++i]
    else if (arg === "--reviewer") parsed.reviewer = argv[++i]
    else if (arg === "--skip") parsed.skip.push(...parseSkipList(argv[++i]))
    else if (arg === "--status") parsed.status = argv[++i]
    else if (arg === "--summary") parsed.summary = argv[++i]
    else fail(`Unknown argument: ${arg}`)
  }
  return parsed
}

function printHelp() {
  process.stdout.write(`Usage:
  node scripts/v3-strict-proof-record.mjs --requirement <name> [options]
  node scripts/v3-strict-proof-record.mjs --list
  node scripts/v3-strict-proof-record.mjs --next
  node scripts/v3-strict-proof-record.mjs --check-ready

Options:
  --plans-dir <path>         Override the V3 Plans directory.
  --list                     Print the current V3 strict proof status without writing files.
  --next                     Print the next open strict proof requirement in closure order.
  --skip <name[,name]>       With --next, ignore user-owned requirements without marking them passed.
  --check-ready              Exit 0 only when all strict proof entries are ready; otherwise exit 2.
  --json                     With --list, --next, or --check-ready, print machine-readable JSON.
  --id <id>                  Stable manifest entry id. Defaults to <requirement>_YYYY_MM_DD.
  --status pending|passed    Defaults to pending.
  --evidence-kind real|sandbox
  --artifact <path>          Relative artifact path under the Plans directory.
  --reviewer <name>          Required when marking passed.
  --summary <text>           Required when marking passed.
  --performed-at <iso-time>  Defaults to now.
  --confirm-reviewed         Required when marking passed.

Requirements:
  ${closureOrder.join("\n  ")}
`)
}

function printNext(manifest, { json, skip }) {
  const status = buildStatusList(manifest)
  const skipped = new Set(skip ?? [])
  const next = status.requirements.find((item) => item.gateStatus !== "passed" && !skipped.has(item.requirement)) ?? null
  if (json) {
    process.stdout.write(
      `${JSON.stringify(
        {
          ready: next === null,
          next: next ? { ...next, helperCommands: helperCommandsForRequirement(next.requirement) } : null,
          skipped: [...skipped],
          summary: status.summary,
        },
        null,
        2,
      )}\n`,
    )
    return
  }

  if (!next) {
    process.stdout.write(
      skipped.size > 0
        ? `All non-skipped V3 strict proof entries are ready. Skipped requirement(s) still need proof before final close: ${[...skipped].join(", ")}.\n`
        : "All V3 strict proof entries are ready. Run scripts/v3-strict-proof-audit.mjs next.\n",
    )
    return
  }

  const artifactPath = next.artifacts[0]?.path ?? defaultArtifactPath(next.requirement)
  const artifactAbsolutePath = next.artifacts[0]?.absolutePath ?? path.resolve(plansDir, artifactPath)
  const command = [
    "node scripts/v3-strict-proof-record.mjs \\",
    `  --requirement ${next.requirement} \\`,
    `  --id ${next.id ?? `${next.requirement}_${new Date().toISOString().slice(0, 10).replaceAll("-", "_")}`} \\`,
    "  --status passed \\",
    `  --artifact ${artifactPath} \\`,
    '  --reviewer "manual:<name>" \\',
    `  --summary "${next.title} proof completed." \\`,
    "  --confirm-reviewed",
  ].join("\n")

  process.stdout.write(
    [
      `Next V3 strict proof requirement: ${next.requirement}`,
      `Title: ${next.title}`,
      `Status: ${next.gateStatus}`,
      `Artifact: ${artifactPath}`,
      `Artifact absolute path: ${artifactAbsolutePath}`,
      skipped.size > 0 ? `Skipped requirement(s): ${[...skipped].join(", ")}` : null,
      `Next action: ${next.next}`,
      "",
      "Suggested evidence helper command(s):",
      ...helperCommandsForRequirement(next.requirement).map((line) => `  ${line}`),
      "",
      "After real execution and checklist completion, mark it passed with:",
      command,
      "",
    ].filter(Boolean).join("\n"),
  )
}

function parseSkipList(value) {
  const names = String(value)
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean)
  for (const name of names) {
    if (!requirements[name]) fail(`Unknown --skip requirement: ${name}`)
  }
  return names
}

function helperCommandsForRequirement(requirement) {
  if (requirement === "tauri_manual_gui_smoke") {
    return [
      'node scripts/v3-tauri-manual-smoke-helper.mjs --append --reviewer "<name>"',
      "node scripts/v3-tauri-smoke-launch.mjs --run",
    ]
  }
  if (requirement === "real_restart_resume_matrix") {
    return [
      'node scripts/v3-restart-resume-reviewer-packet.mjs --append --reviewer "<name>"',
      "node scripts/v3-restart-resume-matrix-runner.mjs --scenario long-workflow --long-sleep-secs 180 --wait-state-secs 25 --wait-health-secs 90 --kill-delay-secs 1 --restart-delay-secs 3 --run-id long-restart-extra-YYYYMMDD --json",
      "node scripts/v3-restart-resume-matrix-runner.mjs --scenario dynamic-loop --wait-health-secs 90 --wait-state-secs 30 --dynamic-loop-fallback-secs 120 --run-id dynamic-loop-extra-YYYYMMDD --json",
    ]
  }
  if (requirement === "real_soak") {
    return [
      'node scripts/v3-wall-clock-soak-helper.mjs --phase start --append --reviewer "<name>"',
      'node scripts/v3-wall-clock-soak-helper.mjs --phase checkpoint --append --start-at <start-iso> --reviewer "<name>" --session-id <id> --goal-id <id> --loop-id <id> --workflow-run-id <id> --notes "<manual observation>"',
      'node scripts/v3-wall-clock-soak-helper.mjs --phase finish --append --start-at <start-iso> --reviewer "<name>" --session-id <id> --goal-id <id> --loop-id <id> --loop-run-id <id> --workflow-run-id <id> --notes "<final observation>"',
    ]
  }
  if (requirement === "connector_readback") {
    return [
      'node scripts/v3-connector-readback-helper.mjs --phase prepare --append --reviewer "<name>" --connector <connector> --account <test-or-sandbox-account> --action "<harmless connector mutation>" --expected-state "<expected read-back state>"',
      'node scripts/v3-connector-readback-helper.mjs --phase finish --append --reviewer "<name>" --connector <connector> --object-id <id> --approval-id <approval-id> --execution-result "<result>" --readback-result "<read-back>" --rollback-result "<cleanup>"',
    ]
  }
  if (requirement === "codex_claude_comparison") {
    return [
      'node scripts/v3-agent-comparison-helper.mjs --phase prepare --append --reviewer "<name>"',
      'node scripts/v3-agent-comparison-helper.mjs --phase result --append --reviewer "<name>" --system hope --run-dir "<clean-hope-dir>" --model "<model>" --permission-mode "<mode>" --completed-required <n> --validation-result "<commands/results>" --manual-smoke "<manual smoke>" --token-cost "<time/token/cost>" --nudges "<count/details>" --recovery-notes "<failures/recovery>"',
      'node scripts/v3-agent-comparison-helper.mjs --phase finish --append --reviewer "<name>" --hope-summary "<Hope vs baselines>" --codex-summary "<Codex/Claude baseline>" --finish-decision "<accept/reject/follow-up>"',
    ]
  }
  return [`node scripts/v3-strict-proof-record.mjs --requirement ${requirement}`]
}

function checkReady(manifest, { json }) {
  const status = buildStatusList(manifest)
  const ready = status.summary.remaining === 0
  if (json) {
    process.stdout.write(`${JSON.stringify({ ready, ...status }, null, 2)}\n`)
  } else {
    process.stdout.write(
      ready
        ? "V3 strict proof is ready for final audit.\n"
        : `V3 strict proof is not ready: ${status.summary.remaining}/${status.summary.total} requirement(s) remaining.\n`,
    )
  }
  process.exit(ready ? 0 : 2)
}

function defaultArtifactPath(requirement) {
  const spec = requirements[requirement]
  const today = new Date().toISOString().slice(0, 10)
  return `evidence/${spec.artifactSlug}-${today}.md`
}

function printStatusList(manifest, { json }) {
  const status = buildStatusList(manifest)
  if (json) {
    process.stdout.write(`${JSON.stringify(status, null, 2)}\n`)
    return
  }

  const lines = []
  lines.push("V3 strict proof status")
  lines.push(`Manifest: ${status.manifest.path}${status.manifest.exists ? "" : " (missing)"}`)
  lines.push(`Updated: ${status.manifest.updatedAt ?? "unknown"}`)
  lines.push(`Summary: ${status.summary.passed}/${status.summary.total} passed, ${status.summary.remaining} remaining`)
  lines.push("")

  for (const item of status.requirements) {
    lines.push(`- ${item.requirement}: ${item.gateStatus}`)
    lines.push(`  id: ${item.id ?? "(missing id)"}`)
    lines.push(`  title: ${item.title}`)
    lines.push(`  artifact: ${item.artifacts.length > 0 ? item.artifacts.map(formatArtifactStatus).join("; ") : "no artifact"}`)
    if (item.issues.length > 0) lines.push(`  issues: ${item.issues.slice(0, 3).join("; ")}`)
    lines.push(`  next: ${item.next}`)
  }

  lines.push("")
  lines.push("Close V3 only after scripts/v3-strict-proof-audit.mjs exits 0.")
  process.stdout.write(`${lines.join("\n")}\n`)
}

function buildStatusList(manifest) {
  const items = closureOrder.map((requirement) => {
    const spec = requirements[requirement]
    const entry = latestEntryForRequirement(manifest, requirement)
    if (!entry) {
      return {
        requirement,
        title: spec.title,
        gateStatus: "missing",
        id: null,
        manifestStatus: null,
        evidenceKind: null,
        artifacts: [],
        next: `create a pending entry with --requirement ${requirement}`,
      }
    }

    const artifacts = Array.isArray(entry.artifacts)
      ? entry.artifacts.map((artifact) => artifactChecklistStatus(artifact, spec))
      : []
    const artifactErrors = artifacts.flatMap((artifact) => artifact.errors)
    const manifestErrors = validateManifestEntryForList(entry, spec)
    const gateStatus =
      entry.status !== "passed"
        ? "pending"
        : manifestErrors.length > 0 || artifactErrors.length > 0
          ? "incomplete"
          : "passed"
    const next =
      gateStatus === "passed"
        ? "run v3-strict-proof-audit.mjs to confirm this requirement passes"
        : "fill the artifact, check all coverage/reviewer boxes, then rerun with --status passed --confirm-reviewed"

    return {
      requirement,
      title: spec.title,
      gateStatus,
      id: entry.id ?? null,
      manifestStatus: entry.status ?? null,
      evidenceKind: entry.evidenceKind ?? null,
      artifacts,
      issues: [...manifestErrors, ...artifactErrors],
      next,
    }
  })

  return {
    manifest: {
      path: manifestPath,
      exists: fs.existsSync(manifestPath),
      updatedAt: manifest.updatedAt ?? null,
    },
    summary: {
      total: items.length,
      passed: items.filter((item) => item.gateStatus === "passed").length,
      remaining: items.filter((item) => item.gateStatus !== "passed").length,
      missing: items.filter((item) => item.gateStatus === "missing").length,
      pending: items.filter((item) => item.gateStatus === "pending").length,
      incomplete: items.filter((item) => item.gateStatus === "incomplete").length,
    },
    requirements: items,
  }
}

function validateManifestEntryForList(entry, spec) {
  const errors = []
  if (!spec.allowedEvidenceKinds.includes(entry.evidenceKind)) {
    errors.push(`evidenceKind must be one of: ${spec.allowedEvidenceKinds.join(", ")}`)
  }
  if (typeof entry.summary !== "string" || !entry.summary.trim()) {
    errors.push("summary must be a non-empty string")
  }
  if (typeof entry.performedAt !== "string" || Number.isNaN(Date.parse(entry.performedAt))) {
    errors.push("performedAt must be an ISO timestamp string")
  }
  if (!Array.isArray(entry.coverage)) {
    errors.push("coverage must be an array")
  } else {
    const missingCoverage = spec.coverage.filter((item) => !entry.coverage.includes(item))
    if (missingCoverage.length > 0) errors.push(`coverage missing: ${missingCoverage.join(", ")}`)
  }
  return errors
}

function formatArtifactStatus(artifact) {
  if (!artifact.exists) return `${artifact.path} (missing)`
  return `${artifact.path} (coverage ${artifact.coverage.checked}/${artifact.coverage.total}, reviewer ${artifact.reviewer.checked}/${artifact.reviewer.total})`
}

function latestEntryForRequirement(manifest, requirement) {
  const entries = manifest.evidence.filter((entry) => entry && entry.requirement === requirement)
  return entries.length > 0 ? entries[entries.length - 1] : null
}

function artifactChecklistStatus(artifact, spec) {
  const rel = normalizeArtifactPath(artifact)
  const file = path.resolve(plansDir, rel)
  if (!fs.existsSync(file)) {
    return {
      path: rel,
      absolutePath: file,
      exists: false,
      coverage: { checked: 0, total: spec.coverage.length },
      reviewer: { checked: 0, total: 3 },
      errors: [`artifact missing: ${rel}`],
    }
  }
  const content = fs.readFileSync(file, "utf8")
  const checkedCoverage = spec.coverage.filter((item) => hasCheckedListItem(content, item)).length
  const decisions = reviewerDecisionLabels
  const checkedDecisions = decisions.filter((item) => hasCheckedListItem(content, item)).length
  const errors = []
  if (checkedCoverage !== spec.coverage.length) errors.push("coverage checklist incomplete")
  if (checkedDecisions !== decisions.length) errors.push("reviewer checklist incomplete")
  return {
    path: rel,
    absolutePath: file,
    exists: true,
    coverage: { checked: checkedCoverage, total: spec.coverage.length },
    reviewer: { checked: checkedDecisions, total: decisions.length },
    errors,
  }
}

function readManifest(file) {
  if (!fs.existsSync(file)) {
    return { schemaVersion: 1, updatedAt: new Date().toISOString(), evidence: [] }
  }
  let parsed = null
  try {
    parsed = JSON.parse(fs.readFileSync(file, "utf8"))
  } catch (error) {
    fail(`Cannot parse existing manifest ${file}: ${error.message}`)
  }
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) fail("Manifest root must be a JSON object.")
  if (parsed.schemaVersion !== 1) fail("Manifest schemaVersion must be 1.")
  if (!Array.isArray(parsed.evidence)) fail("Manifest evidence must be an array.")
  return parsed
}

function writeJson(file, value) {
  fs.writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`)
}

function normalizeArtifactPath(artifact) {
  if (!artifact || typeof artifact !== "string") fail("--artifact must be a non-empty string.")
  const resolved = path.resolve(plansDir, artifact)
  const rel = path.relative(plansDir, resolved)
  if (rel.startsWith("..") || path.isAbsolute(rel)) fail(`Artifact must stay under the Plans directory: ${artifact}`)
  return rel
}

function renderArtifactTemplate({ id, status, spec, artifactRel, reviewer, summary, performedAt }) {
  return `# ${spec.title}

Entry id: ${id}
Status: ${status}
Performed at: ${performedAt}
Reviewer: ${reviewer}
Artifact: ${artifactRel}

## Summary

${summary}

## Environment

- Hope commit:
- App build/version:
- Runtime:
- Workspace/worktree:
- Session id:
- Model:
- Permission mode:
- Sandbox/incognito:

## Required Coverage

${spec.coverage.map((item) => `- [ ] ${item}`).join("\n")}

## Steps

1. 

## Expected Result

- 

## Actual Result

- 

## Evidence

- Screenshots/logs:
- Commands:
- Durable ids:

## Failures And Recovery

- 

## Reviewer Decision

${reviewerDecisionLabels.map((item) => `- [ ] ${item}`).join("\n")}
`
}

function validateStrictArtifactContent(file, requiredCoverage, requirement) {
  const content = fs.readFileSync(file, "utf8")
  const errors = []
  const missingCheckedCoverage = requiredCoverage.filter((item) => !hasCheckedListItem(content, item))
  if (missingCheckedCoverage.length > 0) {
    errors.push(`unchecked coverage: ${missingCheckedCoverage.join(", ")}`)
  }

  const requiredDecisions = reviewerDecisionLabels
  const missingDecisions = requiredDecisions.filter((item) => !hasCheckedListItem(content, item))
  if (missingDecisions.length > 0) {
    errors.push(`unchecked reviewer decision item(s): ${missingDecisions.join(" | ")}`)
  }

  errors.push(...specificStrictArtifactErrors(content, requirement, file))

  return errors
}

function specificStrictArtifactErrors(content, requirement, file) {
  const rel = path.relative(plansDir, file)
  if (requirement === "real_soak") {
    const hasPositiveContinuationCount =
      /"goal_continuation"\s*:\s*\{[\s\S]*?"observed"\s*:\s*true[\s\S]*?"positiveContinuationRunCount"\s*:\s*[1-9]\d*/.test(
        content,
      )
    const hasPositiveModelUsage =
      /"(providerEvents|assistantMessages|providerTotalTokens|totalTokens)"\s*:\s*[1-9]\d*/.test(content)
    const hasRunnerPositive = hasPositiveContinuationCount && hasPositiveModelUsage
    const hasManualPositive = /^-?\s*goal_continuation_model_usage\s*:\s*(passed|true|yes)\s*$/im.test(content)
    if (!hasRunnerPositive && !hasManualPositive) {
      return [
        `artifact ${rel} lacks model-backed Goal continuation evidence; include a runner result with positiveContinuationRunCount > 0 or a reviewer line "goal_continuation_model_usage: passed"`,
      ]
    }
  }

  if (requirement === "connector_readback") {
    const hasHelperFinish =
      content.includes("## Connector Read-back Finish Packet") &&
      /^- connector_execution: present\s*$/m.test(content) &&
      /^- post_action_readback: present\s*$/m.test(content) &&
      /^- approval_or_sandbox: present\s*$/m.test(content) &&
      /^- rollback_or_recovery: present\s*$/m.test(content)
    const hasManualPositive = /^-?\s*connector_readback_e2e\s*:\s*(passed|true|yes)\s*$/im.test(content)
    if (!hasHelperFinish && !hasManualPositive) {
      return [
        `artifact ${rel} lacks connector E2E proof packet; include a finish packet with all connector coverage hints present or a reviewer line "connector_readback_e2e: passed"`,
      ]
    }
  }

  if (requirement === "codex_claude_comparison") {
    const hasResultPackets =
      content.includes("## Hope Result Packet") &&
      content.includes("## Claude Code Result Packet") &&
      content.includes("## Codex Result Packet") &&
      content.includes("## Agent Comparison Finish Packet")
    const hasManualPositive = /^-?\s*agent_comparison_real_runs\s*:\s*(passed|true|yes)\s*$/im.test(content)
    if (!hasResultPackets && !hasManualPositive) {
      return [
        `artifact ${rel} lacks real three-agent comparison packets; include Hope, Claude Code, Codex result packets plus finish packet or a reviewer line "agent_comparison_real_runs: passed"`,
      ]
    }
  }

  return []
}

function hasCheckedListItem(content, label) {
  const escaped = escapeRegExp(label)
  return new RegExp(`^- \\[[xX]\\] ${escaped}\\s*$`, "m").test(content)
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
}

function fail(message) {
  process.stderr.write(`${message}\n`)
  process.exit(1)
}
