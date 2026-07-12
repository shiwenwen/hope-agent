import type { MemoryHealth, MemoryRepairAction } from "./types"

export interface MemoryHealthRepairHint {
  action: MemoryRepairAction
  label: string
  reason: string
}

export type MemoryHealthRepairPolicy = "none" | "direct_repair" | "snapshot_first"

function hasIssue(health: MemoryHealth, code: string): boolean {
  return health.issues.some((issue) => issue.code === code)
}

function quickCheckFailed(health: MemoryHealth): boolean {
  const value = health.quickCheck.trim().toLowerCase()
  return value.length > 0 && value !== "ok" && value !== "not_checked"
}

export function memoryHealthRepairPolicy(health: MemoryHealth): MemoryHealthRepairPolicy {
  if (quickCheckFailed(health) || hasIssue(health, "db_quick_check_failed")) {
    return "snapshot_first"
  }
  return memoryHealthRepairHints(health).length > 0 ? "direct_repair" : "none"
}

export function memoryHealthRepairHints(health: MemoryHealth): MemoryHealthRepairHint[] {
  const hints: MemoryHealthRepairHint[] = []
  const claimGraphOrphans = health.orphanEvidenceRows + health.orphanClaimLinks

  if (quickCheckFailed(health) || hasIssue(health, "db_quick_check_failed")) {
    return [
      {
        action: "create_db_snapshot",
        label: "Create database snapshot",
        reason: "SQLite quick_check reported a database problem; preserve a raw snapshot first.",
      },
    ]
  }

  if (
    health.ftsMissingRows > 0 ||
    hasIssue(health, "memory_fts_missing") ||
    hasIssue(health, "memory_fts_missing_rows")
  ) {
    hints.push({
      action: "rebuild_fts",
      label: "Rebuild keyword index",
      reason: "Legacy memory keyword index is missing rows or its FTS table is unavailable.",
    })
  }

  if (
    health.claimFtsMissingRows > 0 ||
    (health.evidenceFtsMissingRows ?? 0) > 0 ||
    hasIssue(health, "claim_fts_missing") ||
    hasIssue(health, "claim_fts_missing_rows") ||
    hasIssue(health, "evidence_fts_missing") ||
    hasIssue(health, "evidence_fts_missing_rows")
  ) {
    hints.push({
      action: "rebuild_claim_fts",
      label: "Rebuild structured index",
      reason: "Structured memory keyword index is missing rows or its FTS table is unavailable.",
    })
  }

  if (claimGraphOrphans > 0 || hasIssue(health, "orphan_claim_graph_rows")) {
    hints.push({
      action: "repair_claim_graph",
      label: "Repair claim graph links",
      reason: "Structured memory evidence or claim links point at missing rows.",
    })
  }

  if (health.orphanProcedureEpisodeRefs > 0 || hasIssue(health, "orphan_procedure_episode_refs")) {
    hints.push({
      action: "repair_experience_graph",
      label: "Repair experience links",
      reason: "Procedure memory points at missing source episodes.",
    })
  }

  if (
    health.dreamingStaleRuns > 0 ||
    health.dreamingStaleLocks > 0 ||
    hasIssue(health, "dreaming_state_stale")
  ) {
    hints.push({
      action: "recover_dreaming_state",
      label: "Recover Dreaming state",
      reason: "Dreaming maintenance has stale runs or locks.",
    })
  }

  return hints
}
