import { describe, expect, test } from "vitest"
import {
  auditClaimSummary,
  conflictResolutionNote,
  conflictResolverSignal,
} from "./claimConflictAudit"
import type { ClaimRecord } from "./claimTypes"

function claim(patch: Partial<ClaimRecord> = {}): ClaimRecord {
  return {
    id: "claim-current",
    scopeType: "global",
    scopeId: null,
    claimType: "preference",
    subject: "user",
    predicate: "prefers",
    object: "short answers",
    content: "The user prefers short answers.",
    tags: ["style"],
    confidence: 0.82,
    confidenceSource: "derived",
    salience: 0.71,
    status: "needs_review",
    validFrom: null,
    validUntil: null,
    createdAt: "2026-07-07T00:00:00Z",
    updatedAt: "2026-07-07T00:00:00Z",
    ...patch,
  }
}

describe("claimConflictAudit", () => {
  test("summarizes trust, scores, and evidence counts for audit notes", () => {
    expect(
      auditClaimSummary(
        claim(),
        "sourceBacked",
        { confirmed: 1, sourceBacked: 2, inferred: 0 },
        3,
      ),
    ).toBe(
      'claim-current; object="short answers"; status=needs_review; confidence=82%; salience=71%; trust=sourceBacked; evidence=3 (confirmed=1, sourceBacked=2, inferred=0)',
    )
  })

  test("keeps conflict-resolution rationale complete enough for Review History", () => {
    const note = conflictResolutionNote({
      action: "use_current",
      current: claim({ id: "claim-current", object: "Chinese replies", status: "needs_review" }),
      existing: claim({
        id: "claim-existing",
        object: "English replies",
        status: "active",
        confidence: 0.91,
        salience: 0.63,
      }),
      currentTrustKey: "userConfirmed",
      currentStats: { confirmed: 2, sourceBacked: 0, inferred: 1 },
      currentEvidenceCount: 3,
      existingTrustKey: "sourceBacked",
      existingStats: { confirmed: 0, sourceBacked: 2, inferred: 0 },
      existingEvidenceCount: 2,
      activeConflictCount: 1,
      archivedConflictCount: 2,
    })

    expect(note).toContain("enabled the review candidate")
    expect(note).toContain('Current: claim-current; object="Chinese replies"')
    expect(note).toContain("trust=userConfirmed")
    expect(note).toContain("evidence=3 (confirmed=2, sourceBacked=0, inferred=1)")
    expect(note).toContain('Existing: claim-existing; object="English replies"')
    expect(note).toContain("trust=sourceBacked")
    expect(note).toContain(
      "Resolver signal: current candidate stronger; trust userConfirmed vs sourceBacked; confidence -9%; salience +8%; evidence +1.",
    )
    expect(note).toContain("Active conflicts=1.")
    expect(note).toContain("Archived conflicts=2.")
  })

  test("summarizes deterministic resolver signal without exposing evidence text", () => {
    expect(
      conflictResolverSignal({
        action: "keep_existing",
        current: claim({ confidence: 0.72, salience: 0.64 }),
        existing: claim({
          id: "claim-existing",
          object: "long answers",
          status: "active",
          confidence: 0.91,
          salience: 0.8,
        }),
        currentTrustKey: "inferred",
        currentStats: { confirmed: 0, sourceBacked: 0, inferred: 2 },
        currentEvidenceCount: 2,
        existingTrustKey: "userConfirmed",
        existingStats: { confirmed: 1, sourceBacked: 1, inferred: 0 },
        existingEvidenceCount: 4,
      }),
    ).toBe(
      "Resolver signal: existing memory stronger; trust inferred vs userConfirmed; confidence -19%; salience -16%; evidence -2.",
    )
  })

  test("bounds long object text and avoids unsafe quote characters", () => {
    const summary = auditClaimSummary(
      claim({
        object: '"'.repeat(2) + "very ".repeat(40),
      }),
      null,
      null,
      null,
    )

    expect(summary).toContain("object=\"''very very")
    expect(summary).toContain("...")
    expect(summary).toContain("trust=unknown")
    expect(summary).toContain("evidence=unknown")
  })
})
