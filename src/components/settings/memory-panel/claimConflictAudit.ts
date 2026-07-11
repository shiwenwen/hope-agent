import type { ClaimRecord } from "./claimTypes"

export type ConflictResolutionAction = "keep_existing" | "use_current" | "archive_superseded"
export type ConflictResolutionTrustKey =
  | "userCorrected"
  | "userConfirmed"
  | "sourceBacked"
  | "inferred"
  | "weak"
  | string

export interface ConflictEvidenceStats {
  confirmed: number
  inferred: number
  sourceBacked: number
}

export interface ConflictResolutionNoteArgs {
  action: ConflictResolutionAction
  current: ClaimRecord
  existing?: ClaimRecord | null
  currentTrustKey: ConflictResolutionTrustKey | null
  currentStats: ConflictEvidenceStats | null
  currentEvidenceCount: number | null
  existingTrustKey?: ConflictResolutionTrustKey | null
  existingStats?: ConflictEvidenceStats | null
  existingEvidenceCount?: number | null
  activeConflictCount?: number
  archivedConflictCount?: number
}

function auditPercent(value: number): string {
  if (!Number.isFinite(value)) return "unknown"
  return `${Math.round(value * 100)}%`
}

function auditText(value: string | null | undefined, fallback = "unknown"): string {
  const normalized = (value ?? "").replace(/\s+/g, " ").replace(/"/g, "'").trim()
  if (!normalized) return fallback
  return normalized.length > 96 ? `${normalized.slice(0, 93)}...` : normalized
}

function trustRank(trustKey: ConflictResolutionTrustKey | null | undefined): number {
  switch (trustKey) {
    case "userCorrected":
      return 5
    case "userConfirmed":
      return 4
    case "sourceBacked":
      return 3
    case "inferred":
      return 2
    case "weak":
      return 1
    default:
      return 0
  }
}

function signedPercent(delta: number): string {
  if (!Number.isFinite(delta)) return "unknown"
  const rounded = Math.round(delta * 100)
  return rounded >= 0 ? `+${rounded}%` : `${rounded}%`
}

function signedCount(delta: number): string {
  return delta >= 0 ? `+${delta}` : `${delta}`
}

export function conflictResolverSignal(args: ConflictResolutionNoteArgs): string | null {
  if (!args.existing) return null

  const signals: string[] = []
  let currentWins = 0
  let existingWins = 0

  const currentTrust = trustRank(args.currentTrustKey)
  const existingTrust = trustRank(args.existingTrustKey)
  if (currentTrust !== existingTrust) {
    if (currentTrust > existingTrust) currentWins += 1
    else existingWins += 1
    signals.push(
      `trust ${args.currentTrustKey ?? "unknown"} vs ${args.existingTrustKey ?? "unknown"}`,
    )
  }

  const confidenceDelta = args.current.confidence - args.existing.confidence
  if (Math.abs(confidenceDelta) >= 0.05) {
    if (confidenceDelta > 0) currentWins += 1
    else existingWins += 1
    signals.push(`confidence ${signedPercent(confidenceDelta)}`)
  }

  const salienceDelta = args.current.salience - args.existing.salience
  if (Math.abs(salienceDelta) >= 0.05) {
    if (salienceDelta > 0) currentWins += 1
    else existingWins += 1
    signals.push(`salience ${signedPercent(salienceDelta)}`)
  }

  if (args.currentEvidenceCount !== null && args.existingEvidenceCount != null) {
    const evidenceDelta = args.currentEvidenceCount - args.existingEvidenceCount
    if (evidenceDelta !== 0) {
      if (evidenceDelta > 0) currentWins += 1
      else existingWins += 1
      signals.push(`evidence ${signedCount(evidenceDelta)}`)
    }
  }

  if (signals.length === 0) {
    signals.push("balanced trust, score, and evidence signals")
  }

  const leaning =
    currentWins > existingWins
      ? "current candidate stronger"
      : existingWins > currentWins
        ? "existing memory stronger"
        : "mixed evidence"
  return `Resolver signal: ${leaning}; ${signals.join("; ")}.`
}

export function auditClaimSummary(
  claim: Pick<ClaimRecord, "id" | "object" | "status" | "confidence" | "salience">,
  trustKey: ConflictResolutionTrustKey | null,
  stats: ConflictEvidenceStats | null,
  evidenceCount: number | null,
): string {
  const evidence =
    evidenceCount === null
      ? "evidence=unknown"
      : [
          `evidence=${evidenceCount}`,
          stats
            ? `(confirmed=${stats.confirmed}, sourceBacked=${stats.sourceBacked}, inferred=${stats.inferred})`
            : null,
        ]
          .filter(Boolean)
          .join(" ")
  return [
    claim.id,
    `object="${auditText(claim.object)}"`,
    `status=${claim.status}`,
    `confidence=${auditPercent(claim.confidence)}`,
    `salience=${auditPercent(claim.salience)}`,
    `trust=${trustKey ?? "unknown"}`,
    evidence,
  ].join("; ")
}

export function conflictResolutionNote(args: ConflictResolutionNoteArgs): string {
  const action =
    args.action === "keep_existing"
      ? "kept existing memory and archived the review candidate"
      : args.action === "use_current"
        ? "enabled the review candidate and targeted conflicting candidates for archive"
        : "archived a superseded conflicting candidate after enabling the review candidate"
  const parts = [
    `Conflict resolved in Review Inbox: ${action}.`,
    `Current: ${auditClaimSummary(
      args.current,
      args.currentTrustKey,
      args.currentStats,
      args.currentEvidenceCount,
    )}.`,
  ]
  if (args.existing) {
    parts.push(
      `Existing: ${auditClaimSummary(
        args.existing,
        args.existingTrustKey ?? null,
        args.existingStats ?? null,
        args.existingEvidenceCount ?? null,
      )}.`,
    )
  }
  const resolverSignal = conflictResolverSignal(args)
  if (resolverSignal) {
    parts.push(resolverSignal)
  }
  if (typeof args.activeConflictCount === "number") {
    parts.push(`Active conflicts=${args.activeConflictCount}.`)
  }
  if (typeof args.archivedConflictCount === "number") {
    parts.push(`Archived conflicts=${args.archivedConflictCount}.`)
  }
  return parts.join(" ")
}
