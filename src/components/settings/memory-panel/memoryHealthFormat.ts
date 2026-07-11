import type { MemoryHealth, MemoryHealthIssue } from "./types"
import {
  externalMemoryProviderOverview,
  externalMemoryProviderPrivacySummary,
  externalMemoryProviderSyncBlockReasons,
  externalMemoryProviderSupportedSyncPolicies,
} from "./externalMemoryProviderReadiness"
import { memoryHealthRepairHints, memoryHealthRepairPolicy } from "./memoryHealthRepairHints"
import { sanitizeMemoryDiagnosticText } from "./memoryDiagnosticRedaction"

const DIAGNOSTIC_VALUE_MAX_CHARS = 600
const DIAGNOSTIC_LABEL_MAX_CHARS = 160
const DIAGNOSTIC_PROVIDER_ERROR_MAX_CHARS = 440

export type MemoryHealthTranslate = (
  key: string,
  options?: Record<string, unknown>,
) => string

export type DeepResolverHealthTone = "clear" | "backlog" | "blocked"

export interface DeepResolverHealthSummary {
  tone: DeepResolverHealthTone
  statusText: string
  detailText: string | null
  expiredCandidates: number
  conflictGroups: number
  backlogCount: number
  blockingReasons: string[]
}

function yesNo(value: boolean): string {
  return value ? "yes" : "no"
}

function optional(value: string | number | null | undefined): string {
  if (value === null || value === undefined || value === "") return "-"
  return String(value)
}

function diagnosticText(value: string, maxChars = DIAGNOSTIC_VALUE_MAX_CHARS): string {
  return sanitizeMemoryDiagnosticText(value, maxChars)
}

function optionalDiagnostic(
  value: string | null | undefined,
  maxChars = DIAGNOSTIC_VALUE_MAX_CHARS,
): string {
  if (value === null || value === undefined || value.trim() === "") return "-"
  return diagnosticText(value, maxChars)
}

function issueLine(issue: MemoryHealthIssue, index: number): string {
  const action = issue.action ? ` Action: ${diagnosticText(issue.action)}` : ""
  return `${index + 1}. [${issue.severity}] ${diagnosticText(issue.code, DIAGNOSTIC_LABEL_MAX_CHARS)}: ${diagnosticText(issue.message)}${action}`
}

export function deepResolverBlockReasonLabel(
  reason: string,
  t: MemoryHealthTranslate,
): string {
  switch (reason) {
    case "dreaming_disabled":
      return t("settings.memoryHealthDeepResolverReasonDreamingDisabled", {
        defaultValue: "Dreaming off",
      })
    case "long_term_memory_disabled":
      return t("settings.memoryHealthDeepResolverReasonMemoryDisabled", {
        defaultValue: "Memory learning off",
      })
    case "manual_disabled":
      return t("settings.memoryHealthDeepResolverReasonManualDisabled", {
        defaultValue: "Manual runs off",
      })
    case "claim_load_failed":
      return t("settings.memoryHealthDeepResolverReasonClaimLoadFailed", {
        defaultValue: "Claims unavailable",
      })
    default:
      return reason.replace(/_/g, " ")
  }
}

export function formatDeepResolverHealthSummary(
  health: Pick<
    MemoryHealth,
    | "deepResolverExpiredCandidates"
    | "deepResolverConflictGroups"
    | "deepResolverBlockingReasons"
    | "deepResolverGroupsToAnalyze"
    | "deepResolverGroupCap"
    | "deepResolverWouldCallLlm"
  >,
  t: MemoryHealthTranslate,
  labels: { onLabel: string; offLabel: string },
): DeepResolverHealthSummary {
  const expiredCandidates = health.deepResolverExpiredCandidates ?? 0
  const conflictGroups = health.deepResolverConflictGroups ?? 0
  const backlogCount = expiredCandidates + conflictGroups
  const blockingReasons = health.deepResolverBlockingReasons ?? []
  const tone: DeepResolverHealthTone =
    blockingReasons.length > 0 ? "blocked" : backlogCount > 0 ? "backlog" : "clear"

  const statusText =
    blockingReasons.length > 0
      ? t("settings.memoryHealthDeepResolverBlocked", {
          defaultValue: "Deep Resolver blocked: {{reasons}}",
          reasons: blockingReasons
            .map((reason) => deepResolverBlockReasonLabel(reason, t))
            .join(", "),
        })
      : backlogCount > 0
        ? t("settings.memoryHealthDeepResolverBacklog", {
            defaultValue:
              "Deep Resolver backlog: {{expired}} expired · {{groups}} conflict group(s)",
            expired: expiredCandidates,
            groups: conflictGroups,
          })
        : t("settings.memoryHealthDeepResolverClear", {
            defaultValue: "Deep Resolver backlog is clear.",
          })

  const detailText =
    blockingReasons.length === 0
      ? t("settings.memoryHealthDeepResolverDetail", {
          defaultValue: "{{groups}}/{{cap}} group(s) would be analyzed · LLM {{llm}}",
          groups: health.deepResolverGroupsToAnalyze ?? 0,
          cap: health.deepResolverGroupCap ?? 0,
          llm: health.deepResolverWouldCallLlm ? labels.onLabel : labels.offLabel,
        })
      : null

  return {
    tone,
    statusText,
    detailText,
    expiredCandidates,
    conflictGroups,
    backlogCount,
    blockingReasons,
  }
}

export function formatMemoryHealthDiagnostics(health: MemoryHealth): string {
  const externalOverview = externalMemoryProviderOverview(
    health.externalProvidersEnabled,
    health.externalProviders,
  )
  const repairHints = memoryHealthRepairHints(health)
  const repairPolicy = memoryHealthRepairPolicy(health)
  const lines: string[] = [
    "# Memory Health Diagnostics",
    "",
    `- Status: ${health.status}`,
    `- Backend: ${health.backendKind}`,
    `- Checked at: ${health.checkedAt}`,
    `- SQLite quick_check: ${health.quickCheck}`,
    "",
    "## Local Memory",
    "",
    `- Total memories: ${health.totalMemories}`,
    `- Memories with active embedding: ${health.memoriesWithActiveEmbedding}`,
    `- Memories pending embedding: ${health.memoriesPendingEmbedding}`,
    `- Vector rows: ${optional(health.vectorRows)}`,
    `- FTS rows: ${health.ftsRows}`,
    `- FTS missing rows: ${health.ftsMissingRows}`,
    "",
    "## Structured Memory",
    "",
    `- Claims total: ${health.claimsTotal}`,
    `- Claims needing review: ${health.claimsNeedsReview}`,
    `- Claims without evidence: ${health.claimsWithoutEvidence}`,
    `- Claim FTS rows: ${health.claimFtsRows}`,
    `- Claim FTS missing rows: ${health.claimFtsMissingRows}`,
    `- Evidence FTS rows: ${health.evidenceFtsRows ?? 0}`,
    `- Evidence FTS missing rows: ${health.evidenceFtsMissingRows ?? 0}`,
    `- Orphan evidence rows: ${health.orphanEvidenceRows}`,
    `- Orphan claim links: ${health.orphanClaimLinks}`,
    "",
    "## Experience Memory",
    "",
    `- Episodes total: ${health.episodesTotal}`,
    `- Procedures total: ${health.proceduresTotal}`,
    `- Orphan procedure episode refs: ${health.orphanProcedureEpisodeRefs}`,
    "",
    "## Dreaming",
    "",
    `- Running runs: ${health.dreamingRunningRuns}`,
    `- Stale runs: ${health.dreamingStaleRuns}`,
    `- Locks: ${health.dreamingLocks}`,
    `- Stale locks: ${health.dreamingStaleLocks}`,
    `- Deep resolver active claims: ${health.deepResolverActiveClaims ?? 0}`,
    `- Deep resolver expired candidates: ${health.deepResolverExpiredCandidates ?? 0}`,
    `- Deep resolver conflict groups: ${health.deepResolverConflictGroups ?? 0}`,
    `- Deep resolver groups to analyze: ${health.deepResolverGroupsToAnalyze ?? 0}/${health.deepResolverGroupCap ?? 0}`,
    `- Deep resolver would call LLM: ${yesNo(health.deepResolverWouldCallLlm ?? false)}`,
    `- Deep resolver truncated: ${yesNo(health.deepResolverTruncated ?? false)}`,
    `- Deep resolver blocked: ${(health.deepResolverBlockingReasons ?? []).join("|") || "-"}`,
    "",
    "## Latest DB Snapshot",
    "",
  ]

  if (health.latestDbSnapshot) {
    const snapshotFiles = health.latestDbSnapshot.files ?? []
    const snapshotIssues = health.latestDbSnapshot.issues ?? []
    lines.push(
      `- Path: ${health.latestDbSnapshot.path}`,
      `- Created at: ${optional(health.latestDbSnapshot.createdAt)}`,
      `- Status: ${health.latestDbSnapshot.status ?? "ok"}`,
      `- Files: ${snapshotFiles.length}`,
    )
    for (const issue of snapshotIssues) {
      lines.push(`- Issue: ${issue}`)
    }
    for (const file of snapshotFiles) {
      lines.push(`- ${file.name}: ${file.sizeBytes} bytes, sha256=${file.sha256}`)
    }
    lines.push("")
  } else {
    lines.push("- None", "")
  }

  lines.push(
    "## Embedding Provider",
    "",
    `- Configured: ${yesNo(health.embeddingProviderConfigured)}`,
    `- Loaded: ${yesNo(health.embeddingProviderLoaded)}`,
    `- Signature: ${optional(health.activeEmbeddingSignature)}`,
    `- Dimensions: ${optional(health.embeddingProviderDimensions)}`,
    `- Multimodal: ${yesNo(health.embeddingProviderMultimodal)}`,
    `- Batch API: ${yesNo(health.embeddingProviderBatch)}`,
    "",
    "## External Memory Providers",
    "",
    `- Global sync enabled: ${yesNo(health.externalProvidersEnabled)}`,
    `- Configured providers: ${health.externalProviderCount}`,
    `- Active providers: ${health.externalProviderActiveCount}`,
    `- Overview state: ${externalOverview.state}`,
    `- Providers needing setup: ${externalOverview.needsSetupCount}`,
    `- Providers with unsupported policy: ${externalOverview.unsupportedPolicyCount}`,
    `- Providers waiting for adapter: ${externalOverview.adapterPendingCount}`,
    `- Providers with errors: ${externalOverview.errorCount}`,
    `- Provider adapters ready: ${externalOverview.readyCount}`,
  )

  if (health.externalProviders.length > 0) {
    lines.push("")
    for (const provider of health.externalProviders) {
      const privacy = externalMemoryProviderPrivacySummary(
        health.externalProvidersEnabled,
        provider,
      )
      const blockReasons = externalMemoryProviderSyncBlockReasons(
        health.externalProvidersEnabled,
        provider,
      )
      const providerBits = [
        `status=${provider.status}`,
        `enabled=${yesNo(provider.enabled)}`,
        `policy=${provider.syncPolicy}`,
        `policySupported=${yesNo(provider.policySupported ?? true)}`,
        `policyFlow=${privacy.policyDataFlow}`,
        `runtimeFlow=${privacy.runtimeDataFlow}`,
        `outbound=${yesNo(privacy.sendsQueryContext || privacy.sendsLocalMemory)}`,
        `automatic=${yesNo(privacy.automaticSync)}`,
        `blocked=${blockReasons.join("|") || "-"}`,
        `supports=${externalMemoryProviderSupportedSyncPolicies(provider).join("|") || "-"}`,
        `adapter=${yesNo(provider.capabilities?.adapterAvailable ?? false)}`,
        `endpoint=${yesNo(provider.endpointConfigured)}`,
        `lastSync=${optionalDiagnostic(provider.lastSyncAt, DIAGNOSTIC_LABEL_MAX_CHARS)}`,
        `lastError=${optionalDiagnostic(provider.lastError, DIAGNOSTIC_PROVIDER_ERROR_MAX_CHARS)}`,
      ].join(", ")
      lines.push(
        `- ${optionalDiagnostic(provider.displayName, DIAGNOSTIC_LABEL_MAX_CHARS)} (${provider.kind}, ${optionalDiagnostic(provider.id, DIAGNOSTIC_LABEL_MAX_CHARS)}): ${providerBits}`,
      )
    }
  }

  lines.push("", "## Issues", "")
  if (health.issues.length === 0) {
    lines.push("- None")
  } else {
    lines.push(...health.issues.map(issueLine))
  }

  lines.push("", "## Available Repairs", "")
  lines.push(`- Repair policy: ${repairPolicy}`)
  if (repairPolicy === "snapshot_first") {
    lines.push(
      "- Policy note: SQLite integrity check failed; create a database snapshot before running other repairs.",
    )
  }
  if (repairHints.length === 0) {
    lines.push("- None")
  } else {
    for (const hint of repairHints) {
      lines.push(`- ${hint.action}: ${hint.label}. ${hint.reason}`)
    }
  }

  return lines.join("\n")
}
