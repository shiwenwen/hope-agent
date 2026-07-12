import { describe, expect, it } from "vitest"
import { memoryHealthRepairHints, memoryHealthRepairPolicy } from "./memoryHealthRepairHints"
import type { MemoryHealth } from "./types"

function healthFixture(patch: Partial<MemoryHealth> = {}): MemoryHealth {
  return {
    backendKind: "sqlite",
    status: "ok",
    checkedAt: "2026-07-07T10:00:00.000Z",
    quickCheck: "ok",
    totalMemories: 10,
    memoriesWithActiveEmbedding: 10,
    memoriesPendingEmbedding: 0,
    activeEmbeddingSignature: null,
    embeddingProviderConfigured: false,
    embeddingProviderLoaded: false,
    embeddingProviderDimensions: null,
    embeddingProviderMultimodal: false,
    embeddingProviderBatch: false,
    vectorRows: null,
    ftsRows: 10,
    ftsMissingRows: 0,
    claimsTotal: 3,
    claimsNeedsReview: 0,
    claimsWithoutEvidence: 0,
    claimFtsRows: 3,
    claimFtsMissingRows: 0,
    evidenceFtsRows: 0,
    evidenceFtsMissingRows: 0,
    orphanEvidenceRows: 0,
    orphanClaimLinks: 0,
    episodesTotal: 0,
    proceduresTotal: 0,
    orphanProcedureEpisodeRefs: 0,
    dreamingRunningRuns: 0,
    dreamingStaleRuns: 0,
    dreamingLocks: 0,
    dreamingStaleLocks: 0,
    externalProvidersEnabled: false,
    externalProviderCount: 0,
    externalProviderActiveCount: 0,
    externalProviders: [],
    issues: [],
    ...patch,
  }
}

describe("memoryHealthRepairHints", () => {
  it("returns no repair hints for a clean health report", () => {
    expect(memoryHealthRepairHints(healthFixture())).toEqual([])
    expect(memoryHealthRepairPolicy(healthFixture())).toBe("none")
  })

  it("maps health gaps to user-triggered repair actions", () => {
    const hints = memoryHealthRepairHints(
      healthFixture({
        ftsMissingRows: 2,
        claimFtsMissingRows: 1,
        evidenceFtsMissingRows: 1,
        orphanEvidenceRows: 1,
        orphanClaimLinks: 1,
        orphanProcedureEpisodeRefs: 1,
        dreamingStaleRuns: 1,
      }),
    )

    expect(hints.map((hint) => hint.action)).toEqual([
      "rebuild_fts",
      "rebuild_claim_fts",
      "repair_claim_graph",
      "repair_experience_graph",
      "recover_dreaming_state",
    ])
    expect(memoryHealthRepairPolicy(healthFixture({ ftsMissingRows: 2 }))).toBe("direct_repair")
  })

  it("uses stable issue codes when counts are unavailable", () => {
    const hints = memoryHealthRepairHints(
      healthFixture({
        issues: [
          {
            code: "memory_fts_missing_rows",
            severity: "warning",
            message: "legacy FTS rows missing",
          },
          {
            code: "claim_fts_missing_rows",
            severity: "warning",
            message: "claim FTS rows missing",
          },
          {
            code: "evidence_fts_missing_rows",
            severity: "warning",
            message: "evidence FTS rows missing",
          },
          {
            code: "orphan_claim_graph_rows",
            severity: "warning",
            message: "orphan rows",
          },
          {
            code: "orphan_procedure_episode_refs",
            severity: "warning",
            message: "orphan experience rows",
          },
          {
            code: "dreaming_state_stale",
            severity: "warning",
            message: "stale dreaming state",
          },
        ],
      }),
    )

    expect(hints.map((hint) => hint.action)).toEqual([
      "rebuild_fts",
      "rebuild_claim_fts",
      "repair_claim_graph",
      "repair_experience_graph",
      "recover_dreaming_state",
    ])
  })

  it("preserves snapshot-first advice when quick_check is not ok even without issue codes", () => {
    expect(
      memoryHealthRepairHints(
        healthFixture({
          quickCheck: "database disk image is malformed",
          ftsMissingRows: 2,
          claimFtsMissingRows: 1,
        }),
      ).map((hint) => hint.action),
    ).toEqual(["create_db_snapshot"])
    expect(
      memoryHealthRepairPolicy(
        healthFixture({
          quickCheck: "database disk image is malformed",
        }),
      ),
    ).toBe("snapshot_first")
  })

  it("keeps snapshot first when db failure issue code appears with other gaps", () => {
    expect(
      memoryHealthRepairHints(
        healthFixture({
          ftsMissingRows: 2,
          orphanClaimLinks: 1,
          issues: [
            {
              code: "db_quick_check_failed",
              severity: "error",
              message: "quick_check failed",
            },
          ],
        }),
      ).map((hint) => hint.action),
    ).toEqual(["create_db_snapshot"])
  })

  it("does not treat not_checked as corruption", () => {
    expect(memoryHealthRepairHints(healthFixture({ quickCheck: "not_checked" }))).toEqual([])
  })
})
