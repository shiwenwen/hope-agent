import { describe, expect, test } from "vitest"

import { buildClaimFocusState, memoryFocusHash, parseMemoryFocusHash } from "./memoryFocus"

describe("memory focus hash", () => {
  test("round-trips an overview audit-search deep-link", () => {
    const target = {
      kind: "overview" as const,
      auditOpen: true,
      auditAction: "update",
      auditQuery: "release workflow",
    }

    const hash = memoryFocusHash(target)

    expect(hash).toBe("#memory/overview?audit=1&auditAction=update&auditQ=release+workflow")
    expect(parseMemoryFocusHash(hash)).toEqual(target)
  })

  test("keeps invalid overview audit actions out of parsed focus state", () => {
    expect(parseMemoryFocusHash("#memory/overview?audit=1&auditAction=restore&auditQ=claim")).toEqual(
      {
        kind: "overview",
        auditOpen: true,
        auditQuery: "claim",
      },
    )
  })

  test("round-trips a claim deep-link with structured-memory filters", () => {
    const target = {
      kind: "claim" as const,
      id: "claim/alpha beta",
      statusFilter: "needs_review",
      claimType: "profile",
      scopeType: "project",
      scopeId: "project 1",
      confidenceSource: "llm_adjusted",
      evidenceClass: "assistant_inferred",
      evidenceSource: "file",
      claimSort: "confidence_asc",
      claimLoaded: 650,
      query: "release checklist",
      reviewHistory: true,
      reviewHistoryDecisionType: "approve",
      reviewHistoryTimeRange: "7d",
      reviewHistoryScopeType: "agent",
      reviewHistoryScopeId: "agent 1",
      reviewHistoryQuery: "kept existing",
    }

    const hash = memoryFocusHash(target)

    expect(hash).toContain("#memory/claim/claim%2Falpha%20beta?")
    expect(hash).toContain("historyDecision=approve")
    expect(hash).toContain("sort=confidence_asc")
    expect(hash).toContain("loaded=650")
    expect(hash).toContain("historyRange=7d")
    expect(hash).toContain("historyScope=agent%3Aagent+1")
    expect(hash).toContain("historyQ=kept+existing")
    expect(parseMemoryFocusHash(hash)).toEqual(target)
  })

  test("parses a claims-list deep-link with scope and search state", () => {
    expect(
      parseMemoryFocusHash(
        "#memory/claims?status=active&type=project_fact&scope=agent%3Aa1&sort=salience_desc&loaded=420&q=ci+failure",
      ),
    ).toEqual({
      kind: "claims",
      statusFilter: "active",
      claimType: "project_fact",
      scopeType: "agent",
      scopeId: "a1",
      claimSort: "salience_desc",
      claimLoaded: 420,
      query: "ci failure",
    })
  })

  test("omits relevance sort but preserves explicit newest sort in claim links", () => {
    expect(
      memoryFocusHash({
        kind: "claims",
        claimSort: "relevance",
        query: "deploy evidence",
      }),
    ).toBe("#memory/claims?q=deploy+evidence")

    const explicitNewest = memoryFocusHash({
      kind: "claims",
      claimSort: "updated_desc",
      query: "deploy evidence",
    })
    expect(explicitNewest).toContain("sort=updated_desc")
  })

  test("builds claim focus state without dropping review-history filters", () => {
    expect(
      buildClaimFocusState(
        {
          statusFilter: "needs_review",
          reviewHistory: true,
          reviewHistoryDecisionType: "archive",
          reviewHistoryTimeRange: "30d",
          reviewHistoryScopeType: "project",
          reviewHistoryScopeId: "proj-1",
          reviewHistoryQuery: "superseded preference",
          selectedId: "claim-1",
        },
        7,
      ),
    ).toEqual({
      nonce: 8,
      statusFilter: "needs_review",
      reviewHistory: true,
      reviewHistoryDecisionType: "archive",
      reviewHistoryTimeRange: "30d",
      reviewHistoryScopeType: "project",
      reviewHistoryScopeId: "proj-1",
      reviewHistoryQuery: "superseded preference",
      selectedId: "claim-1",
    })
  })

  test("pins review-history deep-links to the review queue", () => {
    expect(
      memoryFocusHash({
        kind: "claims",
        statusFilter: "active",
        reviewHistory: true,
        reviewHistoryDecisionType: "approve",
      }),
    ).toBe("#memory/claims?status=needs_review&history=1&historyDecision=approve")

    expect(parseMemoryFocusHash("#memory/claims?history=1&historyDecision=approve")).toEqual({
      kind: "claims",
      statusFilter: "needs_review",
      reviewHistory: true,
      reviewHistoryDecisionType: "approve",
    })

    expect(parseMemoryFocusHash("#memory/claims?status=active&history=1")).toEqual({
      kind: "claims",
      statusFilter: "needs_review",
      reviewHistory: true,
    })
  })

  test("omits review-history filters when history is closed", () => {
    expect(
      memoryFocusHash({
        kind: "claims",
        reviewHistoryDecisionType: "approve",
        reviewHistoryTimeRange: "7d",
        reviewHistoryScopeType: "global",
        reviewHistoryQuery: "ignored",
      }),
    ).toBe("#memory/claims")
  })

  test("keeps legacy object links valid", () => {
    expect(parseMemoryFocusHash("#memory/memory/42")).toEqual({ kind: "memory", id: 42 })
    expect(parseMemoryFocusHash("#memory/profile")).toEqual({ kind: "profile" })
  })

  test("round-trips experience memory source links", () => {
    expect(memoryFocusHash({ kind: "episode", id: "episode/alpha beta" })).toBe(
      "#memory/episode/episode%2Falpha%20beta",
    )
    expect(parseMemoryFocusHash("#memory/episode/episode%2Falpha%20beta")).toEqual({
      kind: "episode",
      id: "episode/alpha beta",
    })
    expect(memoryFocusHash({ kind: "procedure", id: "procedure/alpha beta" })).toBe(
      "#memory/procedure/procedure%2Falpha%20beta",
    )
    expect(parseMemoryFocusHash("#memory/procedure/procedure%2Falpha%20beta")).toEqual({
      kind: "procedure",
      id: "procedure/alpha beta",
    })
  })
})
