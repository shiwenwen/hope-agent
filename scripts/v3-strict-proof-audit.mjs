#!/usr/bin/env node
import fs from "node:fs"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"
import {
  reviewerDecisionLabels,
  strictProofClosureOrder,
  strictProofRequirements,
} from "./v3-strict-proof-requirements.mjs"

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(__dirname, "..")
const defaultPlansDir = path.join(
  os.homedir(),
  "Library/Mobile Documents/com~apple~CloudDocs/HopeAI/Hope Agent/Plans/hope-agent-control-plane-plans-2026-07-05/11-agent-control-plane-v3-claude-parity",
)

const args = parseArgs(process.argv.slice(2))
const plansDir = path.resolve(args.plansDir ?? process.env.HOPE_V3_PLANS_DIR ?? defaultPlansDir)
const outputPath = args.write ? path.resolve(args.write) : null

const evidenceDir = path.join(plansDir, "evidence")
const repoFiles = walk(repoRoot)
const planFiles = walk(plansDir)
const allFiles = [...repoFiles, ...planFiles]
const strictEvidenceManifest = loadStrictEvidenceManifest(path.join(plansDir, "v3-strict-proof-evidence.json"))

const checks = [
  docCheck({
    id: "architecture_goal_v3",
    title: "Goal v3 architecture is documented",
    file: path.join(repoRoot, "docs/architecture/goal.md"),
    needles: ["Goal v3 Runtime Contract", "Goal Runner v3", "goal_budget_usage_counts_post_goal_turns_and_last_round_tokens"],
    category: "architecture",
    requiredForClose: true,
  }),
  docCheck({
    id: "architecture_loop_v3",
    title: "Loop v3 dynamic/self-paced architecture is documented",
    file: path.join(repoRoot, "docs/architecture/loop.md"),
    needles: ["Loop V3.2", "dynamic self-paced Loop", "LoopRun.usage"],
    category: "architecture",
    requiredForClose: true,
  }),
  docCheck({
    id: "architecture_workflow_v3",
    title: "Workflow v3 usage/recovery architecture is documented",
    file: path.join(repoRoot, "docs/architecture/workflow.md"),
    needles: ["parentInjection", "recovery_runner_claims_and_replays_completed_ops_without_duplicates", "WorkflowRunSnapshot.usage"],
    category: "architecture",
    requiredForClose: true,
  }),
  docCheck({
    id: "roadmap_remaining_blockers",
    title: "V3 roadmap names current closure blockers",
    file: path.join(plansDir, "agent-control-plane-v3-roadmap.md"),
    needles: ["V3 当前剩余清单", "真实 restart/resume matrix", "真实 soak", "GUI 人工验收"],
    category: "planning",
    requiredForClose: true,
  }),
  patternCheck({
    id: "workspace_browser_smoke",
    title: "Workspace/browser smoke screenshots exist",
    patterns: [/workspace-v35-smoke-clean-.*\.png$/],
    category: "deterministic_substitute",
    requiredForClose: false,
  }),
  patternCheck({
    id: "chat_input_browser_smoke",
    title: "Chat input/browser smoke screenshots exist",
    patterns: [/chat-input-v35-smoke-.*\.png$/],
    category: "deterministic_substitute",
    requiredForClose: false,
  }),
  patternCheck({
    id: "loop_gui_smoke",
    title: "Loop GUI smoke screenshot exists",
    patterns: [/loop-v3-gui-smoke-.*\.png$/],
    category: "deterministic_substitute",
    requiredForClose: false,
  }),
  codeCheck({
    id: "workflow_injection_usage_test",
    title: "Workflow injection usage deterministic proof exists in source",
    file: path.join(repoRoot, "crates/ha-core/src/workflow/tests.rs"),
    needles: ["workflow_snapshot_reports_parent_injection_usage_by_workflow_result_message"],
    category: "deterministic_substitute",
    requiredForClose: false,
  }),
  codeCheck({
    id: "loop_trigger_usage_test",
    title: "Loop trigger-turn usage deterministic proof exists in source",
    file: path.join(repoRoot, "crates/ha-core/src/loop_control.rs"),
    needles: ["loop_run_usage_prefers_trigger_message_boundary_over_time_window"],
    category: "deterministic_substitute",
    requiredForClose: false,
  }),
  ...strictProofClosureOrder.map((requirement) => {
    const spec = strictProofRequirements[requirement]
    return manifestCheck({
      id: requirement,
      title: spec.auditTitle,
      requirement,
      allowedEvidenceKinds: spec.allowedEvidenceKinds,
      requiredCoverage: spec.coverage,
      category: "strict_proof",
      requiredForClose: true,
      missingMeans: spec.missingMeans,
    })
  }),
]

const blockers = checks.filter((check) => check.requiredForClose && check.status !== "passed")
const report = {
  generatedAt: new Date().toISOString(),
  repoRoot,
  plansDir,
  status: blockers.length === 0 ? "passed" : "failed",
  summary: {
    total: checks.length,
    passed: checks.filter((check) => check.status === "passed").length,
    missing: checks.filter((check) => check.status === "missing").length,
    partial: checks.filter((check) => check.status === "partial").length,
    blockers: blockers.length,
  },
  strictEvidenceManifest: strictEvidenceManifestSummary(strictEvidenceManifest),
  checks,
  blockers,
}

if (args.json) {
  const body = `${JSON.stringify(report, null, 2)}\n`
  if (outputPath) fs.writeFileSync(outputPath, body)
  else process.stdout.write(body)
} else {
  const body = renderMarkdown(report)
  if (outputPath) fs.writeFileSync(outputPath, body)
  else process.stdout.write(body)
}

process.exitCode = blockers.length === 0 ? 0 : 2

function parseArgs(argv) {
  const parsed = { json: false, write: null, plansDir: null }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === "--json") parsed.json = true
    else if (arg === "--write") parsed.write = argv[++i]
    else if (arg === "--plans-dir") parsed.plansDir = argv[++i]
    else if (arg === "--help" || arg === "-h") {
      process.stdout.write(`Usage: node scripts/v3-strict-proof-audit.mjs [--json] [--write path] [--plans-dir path]\n`)
      process.exit(0)
    } else {
      throw new Error(`Unknown argument: ${arg}`)
    }
  }
  return parsed
}

function docCheck(input) {
  return fileNeedleCheck(input)
}

function codeCheck(input) {
  return fileNeedleCheck(input)
}

function fileNeedleCheck({ id, title, file, needles, category, requiredForClose }) {
  const exists = fs.existsSync(file)
  const content = exists ? fs.readFileSync(file, "utf8") : ""
  const missingNeedles = needles.filter((needle) => !content.includes(needle))
  const status = !exists ? "missing" : missingNeedles.length === 0 ? "passed" : "partial"
  return {
    id,
    title,
    category,
    requiredForClose,
    status,
    evidence: exists ? [relativePath(file)] : [],
    missing: exists ? missingNeedles : [`Missing file: ${relativePath(file)}`],
  }
}

function patternCheck({ id, title, patterns, category, requiredForClose, missingMeans }) {
  const evidence = allFiles
    .filter((file) => patterns.some((pattern) => pattern.test(relativePath(file))))
    .map(relativePath)
    .sort()
  return {
    id,
    title,
    category,
    requiredForClose,
    status: evidence.length > 0 ? "passed" : "missing",
    evidence,
    missing: evidence.length > 0 ? [] : [missingMeans ?? "No matching evidence file found."],
  }
}

function manifestCheck({
  id,
  title,
  requirement,
  allowedEvidenceKinds,
  requiredCoverage,
  category,
  requiredForClose,
  missingMeans,
}) {
  const manifestLabel = relativePath(strictEvidenceManifest.path)
  if (!strictEvidenceManifest.found) {
    return {
      id,
      title,
      category,
      requiredForClose,
      status: "missing",
      evidence: [],
      missing: [`Missing strict evidence manifest: ${manifestLabel}`, missingMeans],
    }
  }

  if (strictEvidenceManifest.errors.length > 0) {
    return {
      id,
      title,
      category,
      requiredForClose,
      status: "partial",
      evidence: [manifestLabel],
      missing: strictEvidenceManifest.errors,
    }
  }

  const candidates = strictEvidenceManifest.entries
    .filter((entry) => entry && entry.requirement === requirement)
    .map((entry) =>
      validateStrictEvidenceEntry(entry, {
        allowedEvidenceKinds,
        requirement,
        requiredCoverage,
      }),
    )

  if (candidates.length === 0) {
    return {
      id,
      title,
      category,
      requiredForClose,
      status: "missing",
      evidence: [manifestLabel],
      missing: [`No manifest evidence entry for requirement: ${requirement}.`, missingMeans],
    }
  }

  const passed = candidates.filter((candidate) => candidate.status === "passed")
  if (passed.length > 0) {
    return {
      id,
      title,
      category,
      requiredForClose,
      status: "passed",
      evidence: passed.flatMap((candidate) => candidate.evidence),
      missing: [],
    }
  }

  return {
    id,
    title,
    category,
    requiredForClose,
    status: "partial",
    evidence: candidates.flatMap((candidate) => candidate.evidence),
    missing: candidates.flatMap((candidate) =>
      candidate.errors.map((error) => `Entry ${candidate.id}: ${error}`),
    ),
  }
}

function loadStrictEvidenceManifest(manifestPath) {
  const manifest = {
    path: manifestPath,
    found: false,
    schemaVersion: null,
    updatedAt: null,
    entries: [],
    errors: [],
  }

  if (!fs.existsSync(manifestPath)) return manifest

  manifest.found = true
  let parsed = null
  try {
    parsed = JSON.parse(fs.readFileSync(manifestPath, "utf8"))
  } catch (error) {
    manifest.errors.push(`Invalid JSON in ${relativePath(manifestPath)}: ${error.message}`)
    return manifest
  }

  if (!isPlainObject(parsed)) {
    manifest.errors.push("Manifest root must be a JSON object.")
    return manifest
  }

  manifest.schemaVersion = parsed.schemaVersion ?? null
  manifest.updatedAt = parsed.updatedAt ?? null

  if (parsed.schemaVersion !== 1) {
    manifest.errors.push("Manifest schemaVersion must be 1.")
  }
  if (typeof parsed.updatedAt !== "string" || Number.isNaN(Date.parse(parsed.updatedAt))) {
    manifest.errors.push("Manifest updatedAt must be an ISO timestamp string.")
  }
  if (!Array.isArray(parsed.evidence)) {
    manifest.errors.push("Manifest evidence must be an array.")
    return manifest
  }

  manifest.entries = parsed.evidence
  return manifest
}

function strictEvidenceManifestSummary(manifest) {
  return {
    path: relativePath(manifest.path),
    found: manifest.found,
    schemaVersion: manifest.schemaVersion,
    updatedAt: manifest.updatedAt,
    entries: manifest.entries.length,
    errors: manifest.errors,
  }
}

function validateStrictEvidenceEntry(entry, { allowedEvidenceKinds, requirement, requiredCoverage }) {
  const errors = []
  const entryId = typeof entry?.id === "string" && entry.id.trim() ? entry.id.trim() : "(missing id)"
  const evidence = [`${relativePath(strictEvidenceManifest.path)}#${entryId}`]

  if (!isPlainObject(entry)) {
    return {
      id: entryId,
      status: "partial",
      evidence,
      errors: ["Entry must be a JSON object."],
    }
  }

  if (typeof entry.id !== "string" || !entry.id.trim()) errors.push("id must be a non-empty string.")
  if (entry.status !== "passed") errors.push('status must be "passed".')
  if (!allowedEvidenceKinds.includes(entry.evidenceKind)) {
    errors.push(`evidenceKind must be one of: ${allowedEvidenceKinds.join(", ")}.`)
  }
  if (typeof entry.summary !== "string" || !entry.summary.trim()) {
    errors.push("summary must be a non-empty string.")
  }
  if (typeof entry.performedAt !== "string" || Number.isNaN(Date.parse(entry.performedAt))) {
    errors.push("performedAt must be an ISO timestamp string.")
  }

  if (!Array.isArray(entry.artifacts) || entry.artifacts.length === 0) {
    errors.push("artifacts must be a non-empty array.")
  } else {
    for (const artifact of entry.artifacts) {
      const resolved = resolveManifestArtifact(artifact)
      if (!resolved.ok) {
        errors.push(resolved.error)
      } else if (!fs.existsSync(resolved.path)) {
        errors.push(`artifact does not exist: ${artifact}`)
      } else {
        evidence.push(relativePath(resolved.path))
        errors.push(...validateStrictArtifactContent(resolved.path, requiredCoverage, requirement))
      }
    }
  }

  const coverage = Array.isArray(entry.coverage) ? entry.coverage : []
  if (!Array.isArray(entry.coverage)) errors.push("coverage must be an array.")
  const missingCoverage = requiredCoverage.filter((item) => !coverage.includes(item))
  if (missingCoverage.length > 0) {
    errors.push(`coverage is missing: ${missingCoverage.join(", ")}.`)
  }

  return {
    id: entryId,
    status: errors.length === 0 ? "passed" : "partial",
    evidence,
    errors,
  }
}

function resolveManifestArtifact(artifact) {
  if (typeof artifact !== "string" || !artifact.trim()) {
    return { ok: false, error: "artifact path must be a non-empty string." }
  }
  const resolved = path.resolve(plansDir, artifact)
  const rel = path.relative(plansDir, resolved)
  if (rel.startsWith("..") || path.isAbsolute(rel)) {
    return { ok: false, error: `artifact must stay under plansDir: ${artifact}` }
  }
  return { ok: true, path: resolved }
}

function validateStrictArtifactContent(file, requiredCoverage, requirement) {
  const content = fs.readFileSync(file, "utf8")
  const errors = []
  const missingCheckedCoverage = requiredCoverage.filter((item) => !hasCheckedListItem(content, item))
  if (missingCheckedCoverage.length > 0) {
    errors.push(`artifact ${relativePath(file)} has unchecked coverage: ${missingCheckedCoverage.join(", ")}.`)
  }

  const requiredDecisions = reviewerDecisionLabels
  const missingDecisions = requiredDecisions.filter((item) => !hasCheckedListItem(content, item))
  if (missingDecisions.length > 0) {
    errors.push(`artifact ${relativePath(file)} has unchecked reviewer decision item(s): ${missingDecisions.join(" | ")}.`)
  }

  errors.push(...specificStrictArtifactErrors(content, requirement, file))

  return errors
}

function specificStrictArtifactErrors(content, requirement, file) {
  const rel = relativePath(file)
  if (requirement === "real_soak") {
    const hasPositiveContinuationCount =
      /"goal_continuation"\s*:\s*\{[\s\S]*?"observed"\s*:\s*true[\s\S]*?"positiveContinuationRunCount"\s*:\s*[1-9]\d*/.test(
        content,
      )
    const hasPositiveModelUsage =
      /"(providerEvents|assistantMessages|providerTotalTokens|totalTokens)"\s*:\s*[1-9]\d*/.test(content)
    const hasRunnerPositive = hasPositiveContinuationCount && hasPositiveModelUsage
    const hasManualPositive = /^-?\s*goal_continuation_model_usage\s*:\s*(passed|true|yes)\s*$/im.test(content)
    return hasRunnerPositive || hasManualPositive
      ? []
      : [
          `artifact ${rel} lacks model-backed Goal continuation evidence; include a runner result with positiveContinuationRunCount > 0 or a reviewer line "goal_continuation_model_usage: passed".`,
        ]
  }

  if (requirement === "connector_readback") {
    const hasHelperFinish =
      content.includes("## Connector Read-back Finish Packet") &&
      /^- connector_execution: present\s*$/m.test(content) &&
      /^- post_action_readback: present\s*$/m.test(content) &&
      /^- approval_or_sandbox: present\s*$/m.test(content) &&
      /^- rollback_or_recovery: present\s*$/m.test(content)
    const hasManualPositive = /^-?\s*connector_readback_e2e\s*:\s*(passed|true|yes)\s*$/im.test(content)
    return hasHelperFinish || hasManualPositive
      ? []
      : [
          `artifact ${rel} lacks connector E2E proof packet; include a finish packet with all connector coverage hints present or a reviewer line "connector_readback_e2e: passed".`,
        ]
  }

  if (requirement === "codex_claude_comparison") {
    const hasResultPackets =
      content.includes("## Hope Result Packet") &&
      content.includes("## Claude Code Result Packet") &&
      content.includes("## Codex Result Packet") &&
      content.includes("## Agent Comparison Finish Packet")
    const hasManualPositive = /^-?\s*agent_comparison_real_runs\s*:\s*(passed|true|yes)\s*$/im.test(content)
    return hasResultPackets || hasManualPositive
      ? []
      : [
          `artifact ${rel} lacks real three-agent comparison packets; include Hope, Claude Code, Codex result packets plus finish packet or a reviewer line "agent_comparison_real_runs: passed".`,
        ]
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

function isPlainObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value)
}

function walk(root) {
  if (!fs.existsSync(root)) return []
  const out = []
  const stack = [root]
  while (stack.length > 0) {
    const current = stack.pop()
    let entries = []
    try {
      entries = fs.readdirSync(current, { withFileTypes: true })
    } catch {
      continue
    }
    for (const entry of entries) {
      if (entry.name === "node_modules" || entry.name === ".git" || entry.name === "target") continue
      const full = path.join(current, entry.name)
      if (entry.isDirectory()) stack.push(full)
      else if (entry.isFile()) out.push(full)
    }
  }
  return out
}

function relativePath(file) {
  const roots = [repoRoot, plansDir]
  for (const root of roots) {
    const rel = path.relative(root, file)
    if (!rel.startsWith("..") && !path.isAbsolute(rel)) return rel
  }
  return file
}

function renderMarkdown(result) {
  const lines = []
  lines.push("# V3 Strict Proof Audit")
  lines.push("")
  lines.push(`Generated: ${result.generatedAt}`)
  lines.push("")
  lines.push(`Status: **${result.status}**`)
  lines.push("")
  lines.push(
    `Summary: ${result.summary.passed}/${result.summary.total} passed, ${result.summary.blockers} blocking requirement(s) open.`,
  )
  lines.push("")
  lines.push(
    `Strict evidence manifest: ${result.strictEvidenceManifest.found ? "found" : "missing"} (${result.strictEvidenceManifest.path})`,
  )
  if (result.strictEvidenceManifest.found) {
    lines.push(
      `Manifest entries: ${result.strictEvidenceManifest.entries}; updatedAt: ${result.strictEvidenceManifest.updatedAt ?? "unknown"}`,
    )
    for (const error of result.strictEvidenceManifest.errors) lines.push(`- Manifest error: ${error}`)
  }
  lines.push("")
  lines.push("## Blocking Requirements")
  lines.push("")
  if (result.blockers.length === 0) {
    lines.push("- None.")
  } else {
    for (const check of result.blockers) {
      lines.push(`- **${check.id}**: ${check.title}`)
      for (const missing of check.missing) lines.push(`  - ${missing}`)
    }
  }
  lines.push("")
  lines.push("## Checks")
  lines.push("")
  lines.push("| Status | Required | Category | Check | Evidence |")
  lines.push("| --- | --- | --- | --- | --- |")
  for (const check of result.checks) {
    const evidence = check.evidence.length > 0 ? check.evidence.join("<br>") : check.missing.join("<br>")
    lines.push(
      `| ${check.status} | ${check.requiredForClose ? "yes" : "no"} | ${check.category} | ${check.id}: ${check.title} | ${evidence} |`,
    )
  }
  lines.push("")
  lines.push("## Methodology")
  lines.push("")
  lines.push("- This audit only verifies that the expected evidence artifacts and source-level deterministic proofs exist.")
  lines.push(
    "- Strict proof checks require a valid `v3-strict-proof-evidence.json` manifest entry with `status: \"passed\"`, an allowed `evidenceKind`, required coverage labels, and existing artifact paths under the Plans directory.",
  )
  lines.push("- Some strict proof items also require content-level evidence markers so checklist ticks cannot turn calibration or scheduler-only samples into passed proof.")
  lines.push("- Filename matches are used only for deterministic substitutes and display context; they never satisfy strict proof.")
  lines.push("- It intentionally does not turn deterministic substitutes into strict proof.")
  lines.push("- A failed status is expected until every strict proof requirement currently listed as pending in the manifest is attached and marked passed.")
  lines.push("")
  return `${lines.join("\n")}\n`
}
