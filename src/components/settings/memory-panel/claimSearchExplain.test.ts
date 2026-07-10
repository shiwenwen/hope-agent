import { describe, expect, it } from "vitest"
import {
  claimListBackendSortArg,
  claimListSortRuntimeMode,
  claimSearchDiagnostics,
  explainClaimSearchRankSignals,
  explainClaimSearchMatch,
} from "./claimSearchExplain"
import type { ClaimRecord } from "./claimTypes"

function claim(patch: Partial<ClaimRecord> = {}): ClaimRecord {
  return {
    id: "claim-1",
    scopeType: "project",
    scopeId: "project-42",
    claimType: "project_fact",
    subject: "runtime",
    predicate: "uses",
    object: "snake_case_identifier",
    content: "项目使用中文检索",
    tags: ["检索", "retrieval"],
    confidence: 0.7,
    confidenceSource: "derived",
    salience: 0.8,
    status: "needs_review",
    validFrom: null,
    validUntil: null,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    ...patch,
  }
}

describe("explainClaimSearchMatch", () => {
  it("explains CJK content and tag matches", () => {
    expect(
      explainClaimSearchMatch(claim(), "中文 检索", "Project: Hope").map((m) => m.kind),
    ).toEqual(["content", "tag"])
  })

  it("groups subject, predicate, and object matches as triple matches", () => {
    expect(explainClaimSearchMatch(claim(), "snake_case", "Project: Hope")).toEqual([
      { kind: "triple" },
    ])
  })

  it("explains type, status, scope, and confidence source matches", () => {
    expect(
      explainClaimSearchMatch(
        claim({ claimType: "preference", status: "active", confidenceSource: "user_confirmed" }),
        "preference active hope user_confirmed",
        "Project: Hope",
      ).map((m) => m.kind),
    ).toEqual(["type", "status", "scope", "confidence"])
  })

  it("falls back to evidence when the row itself has no visible match", () => {
    expect(explainClaimSearchMatch(claim(), "quoted-source", "Project: Hope")).toEqual([
      { kind: "evidence" },
    ])
  })
})

describe("claimListBackendSortArg", () => {
  it("lets the backend apply relevance when the default search sort is active", () => {
    expect(claimListBackendSortArg("relevance", "quoted source")).toBeUndefined()
  })

  it("falls back to newest updated when relevance has no query to rank", () => {
    expect(claimListBackendSortArg("relevance", " ")).toBe("updated_desc")
  })

  it("preserves explicit advanced sorts even while searching", () => {
    expect(claimListBackendSortArg("confidence_desc", "quoted source")).toBe("confidence_desc")
  })

  it("treats unknown sorts as auto relevance for search and newest otherwise", () => {
    expect(claimListBackendSortArg("mystery", "quoted source")).toBeUndefined()
    expect(claimListBackendSortArg("mystery", "")).toBe("updated_desc")
  })
})

describe("claimListSortRuntimeMode", () => {
  it("explains when best-match ranking is active", () => {
    expect(claimListSortRuntimeMode("relevance", "quoted source")).toBe("best_match")
    expect(claimListSortRuntimeMode("", "quoted source")).toBe("best_match")
  })

  it("explains why relevance falls back without a query", () => {
    expect(claimListSortRuntimeMode("relevance", "")).toBe("recent_fallback")
  })

  it("identifies explicit advanced sorts", () => {
    expect(claimListSortRuntimeMode("salience_desc", "quoted source")).toBe("explicit_sort")
  })
})

describe("explainClaimSearchRankSignals", () => {
  it("shows best-match tie breakers when search relevance is active", () => {
    expect(explainClaimSearchRankSignals("relevance", "release metadata", claim())).toEqual([
      { kind: "salience", direction: "desc", value: 0.8 },
      { kind: "confidence", direction: "desc", value: 0.7 },
      { kind: "updated", direction: "desc", value: "2026-01-01T00:00:00Z" },
    ])
  })

  it("shows newest-updated fallback when relevance has no query", () => {
    expect(explainClaimSearchRankSignals("relevance", " ", claim())).toEqual([
      { kind: "updated", direction: "desc", value: "2026-01-01T00:00:00Z" },
    ])
  })

  it("shows explicit advanced sort signals with deterministic tie breakers", () => {
    expect(explainClaimSearchRankSignals("confidence_asc", "release metadata", claim())).toEqual([
      { kind: "confidence", direction: "asc", value: 0.7 },
      { kind: "updated", direction: "desc", value: "2026-01-01T00:00:00Z" },
    ])
    expect(explainClaimSearchRankSignals("created_desc", "release metadata", claim())).toEqual([
      { kind: "created", direction: "desc", value: "2026-01-01T00:00:00Z" },
      { kind: "updated", direction: "desc", value: "2026-01-01T00:00:00Z" },
    ])
  })

  it("treats unknown sorts like auto relevance for search and newest otherwise", () => {
    expect(
      explainClaimSearchRankSignals("mystery", "release metadata", claim()).map((s) => s.kind),
    ).toEqual(["salience", "confidence", "updated"])
    expect(explainClaimSearchRankSignals("mystery", "", claim())).toEqual([
      { kind: "updated", direction: "desc", value: "2026-01-01T00:00:00Z" },
    ])
  })
})

describe("claimSearchDiagnostics", () => {
  it("combines search matches, runtime mode, and rank signals for copied diagnostics", () => {
    const diagnostics = claimSearchDiagnostics(
      claim(),
      "中文 retrieval",
      "Project: Hope",
      "relevance",
    )

    expect(diagnostics?.query).toBe("中文 retrieval")
    expect(diagnostics?.runtimeMode).toBe("best_match")
    expect(diagnostics?.matches.map((m) => m.kind)).toEqual(["content", "tag"])
    expect(diagnostics?.rankSignals.map((s) => s.kind)).toEqual([
      "salience",
      "confidence",
      "updated",
    ])
  })

  it("omits diagnostics when there is no active search query", () => {
    expect(claimSearchDiagnostics(claim(), " ", "Project: Hope", "relevance")).toBeNull()
  })
})
