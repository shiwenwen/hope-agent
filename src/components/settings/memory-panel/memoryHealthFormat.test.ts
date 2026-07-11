import { describe, expect, it } from "vitest"
import {
  formatDeepResolverHealthSummary,
  formatMemoryHealthDiagnostics,
} from "./memoryHealthFormat"
import type { MemoryHealth } from "./types"

function t(key: string, options?: Record<string, unknown> | string): string {
  if (typeof options === "string") return options
  const template =
    typeof options?.defaultValue === "string" ? options.defaultValue : key
  return template.replace(/\{\{\s*(\w+)\s*\}\}/g, (_match, name: string) =>
    String(options?.[name] ?? ""),
  )
}

function healthFixture(): MemoryHealth {
  return {
    backendKind: "sqlite",
    status: "warning",
    checkedAt: "2026-07-07T10:00:00.000Z",
    quickCheck: "ok",
    totalMemories: 12,
    memoriesWithActiveEmbedding: 8,
    memoriesPendingEmbedding: 4,
    activeEmbeddingSignature: "openai:text-embedding-3-small:1536",
    embeddingProviderConfigured: true,
    embeddingProviderLoaded: true,
    embeddingProviderDimensions: 1536,
    embeddingProviderMultimodal: false,
    embeddingProviderBatch: true,
    vectorRows: 8,
    ftsRows: 11,
    ftsMissingRows: 1,
    claimsTotal: 6,
    claimsNeedsReview: 2,
    claimsWithoutEvidence: 1,
    claimFtsRows: 5,
    claimFtsMissingRows: 1,
    evidenceFtsRows: 7,
    evidenceFtsMissingRows: 1,
    orphanEvidenceRows: 0,
    orphanClaimLinks: 1,
    episodesTotal: 3,
    proceduresTotal: 2,
    orphanProcedureEpisodeRefs: 1,
    dreamingRunningRuns: 1,
    dreamingStaleRuns: 1,
    dreamingLocks: 1,
    dreamingStaleLocks: 1,
    deepResolverActiveClaims: 6,
    deepResolverExpiredCandidates: 1,
    deepResolverConflictGroups: 2,
    deepResolverGroupsToAnalyze: 2,
    deepResolverGroupCap: 50,
    deepResolverTruncated: false,
    deepResolverWouldCallLlm: true,
    deepResolverBlockingReasons: [],
    externalProvidersEnabled: true,
    externalProviderCount: 1,
    externalProviderActiveCount: 1,
    externalProviders: [
      {
        id: "mem0-1",
        kind: "mem0",
        displayName: "Mem0",
        enabled: true,
        syncPolicy: "manual",
        status: "warning",
        capabilities: {
          adapterAvailable: false,
          requiresEndpoint: true,
          supportsManual: true,
          supportsPull: true,
          supportsPush: true,
          supportsBidirectional: true,
        },
        endpointConfigured: false,
        lastSyncAt: null,
        lastError: "missing endpoint",
      },
    ],
    issues: [
      {
        code: "memory_fts_missing_rows",
        severity: "warning",
        message: "1 memory row is missing from the keyword index.",
        action: "Rebuild keyword index.",
      },
    ],
  }
}

describe("formatMemoryHealthDiagnostics", () => {
  it("includes local, structured, experience, dreaming, provider, and issue sections", () => {
    const markdown = formatMemoryHealthDiagnostics(healthFixture())

    expect(markdown).toContain("# Memory Health Diagnostics")
    expect(markdown).toContain("- Status: warning")
    expect(markdown).toContain("- Total memories: 12")
    expect(markdown).toContain("- Claim FTS missing rows: 1")
    expect(markdown).toContain("- Evidence FTS missing rows: 1")
    expect(markdown).toContain("- Orphan procedure episode refs: 1")
    expect(markdown).toContain("- Stale locks: 1")
    expect(markdown).toContain("- Deep resolver active claims: 6")
    expect(markdown).toContain("- Deep resolver expired candidates: 1")
    expect(markdown).toContain("- Deep resolver conflict groups: 2")
    expect(markdown).toContain("- Deep resolver groups to analyze: 2/50")
    expect(markdown).toContain("- Deep resolver would call LLM: yes")
    expect(markdown).toContain("- Deep resolver blocked: -")
    expect(markdown).toContain("- Signature: openai:text-embedding-3-small:1536")
    expect(markdown).toContain("- Overview state: needs_setup")
    expect(markdown).toContain("- Providers needing setup: 1")
    expect(markdown).toContain("- Providers with unsupported policy: 0")
    expect(markdown).toContain("- Providers waiting for adapter: 0")
    expect(markdown).toContain("- Providers with errors: 0")
    expect(markdown).toContain("- Provider adapters ready: 0")
    expect(markdown).toContain(
      "- Mem0 (mem0, mem0-1): status=warning, enabled=yes, policy=manual, policySupported=yes, policyFlow=manual, runtimeFlow=none, outbound=no, automatic=no, blocked=endpoint_missing|adapter_unavailable|last_error, supports=manual|pull_only|push_only|bidirectional, adapter=no",
    )
    expect(markdown).toContain(
      "1. [warning] memory_fts_missing_rows: 1 memory row is missing from the keyword index.",
    )
    expect(markdown).toContain("## Available Repairs")
    expect(markdown).toContain("- Repair policy: direct_repair")
    expect(markdown).toContain("- rebuild_fts: Rebuild keyword index.")
    expect(markdown).toContain("- rebuild_claim_fts: Rebuild structured index.")
    expect(markdown).toContain("- repair_claim_graph: Repair claim graph links.")
    expect(markdown).toContain("- repair_experience_graph: Repair experience links.")
    expect(markdown).toContain("- recover_dreaming_state: Recover Dreaming state.")
  })

  it("prints a clear empty issues section", () => {
    const fixture = healthFixture()
    fixture.status = "ok"
    fixture.issues = []
    fixture.ftsMissingRows = 0
    fixture.claimFtsMissingRows = 0
    fixture.evidenceFtsMissingRows = 0
    fixture.orphanClaimLinks = 0
    fixture.orphanProcedureEpisodeRefs = 0
    fixture.dreamingStaleRuns = 0
    fixture.dreamingStaleLocks = 0

    const markdown = formatMemoryHealthDiagnostics(fixture)

    expect(markdown).toContain("## Issues\n\n- None")
    expect(markdown).toContain("## Available Repairs\n\n- Repair policy: none\n- None")
  })

  it("uses the shared provider overview state in copied diagnostics", () => {
    const fixture = healthFixture()
    fixture.externalProviders[0] = {
      ...fixture.externalProviders[0],
      endpointConfigured: true,
      capabilities: {
        adapterAvailable: true,
        requiresEndpoint: true,
        supportsManual: true,
        supportsPull: true,
        supportsPush: true,
        supportsBidirectional: true,
      },
      lastError: "timeout",
    }

    const markdown = formatMemoryHealthDiagnostics(fixture)

    expect(markdown).toContain("- Overview state: error")
    expect(markdown).toContain("- Providers needing setup: 0")
    expect(markdown).toContain("- Providers with unsupported policy: 0")
    expect(markdown).toContain("- Providers waiting for adapter: 0")
    expect(markdown).toContain("- Providers with errors: 1")
  })

  it("includes runtime privacy flow for ready external providers", () => {
    const fixture = healthFixture()
    fixture.externalProviders[0] = {
      ...fixture.externalProviders[0],
      syncPolicy: "pull_only",
      endpointConfigured: true,
      status: "ok",
      capabilities: {
        adapterAvailable: true,
        requiresEndpoint: true,
        supportsManual: true,
        supportsPull: true,
        supportsPush: true,
        supportsBidirectional: true,
      },
      lastError: null,
    }

    const markdown = formatMemoryHealthDiagnostics(fixture)

    expect(markdown).toContain("policy=pull_only")
    expect(markdown).toContain("policyFlow=pull_only")
    expect(markdown).toContain("runtimeFlow=pull_only")
    expect(markdown).toContain("outbound=yes")
    expect(markdown).toContain("automatic=yes")
    expect(markdown).toContain("blocked=-")
  })

  it("includes latest DB snapshot verification metadata when present", () => {
    const fixture = healthFixture()
    fixture.latestDbSnapshot = {
      path: "/tmp/hope/memory-repair-snapshots/20260707T100000Z",
      createdAt: "2026-07-07T10:00:00.000Z",
      status: "ok",
      issues: [],
      files: [
        {
          name: "memory.db",
          sizeBytes: 4096,
          sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        },
      ],
    }

    const markdown = formatMemoryHealthDiagnostics(fixture)

    expect(markdown).toContain("## Latest DB Snapshot")
    expect(markdown).toContain("- Path: /tmp/hope/memory-repair-snapshots/20260707T100000Z")
    expect(markdown).toContain("- Created at: 2026-07-07T10:00:00.000Z")
    expect(markdown).toContain("- Status: ok")
    expect(markdown).toContain("- Files: 1")
    expect(markdown).toContain(
      "- memory.db: 4096 bytes, sha256=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    )
  })

  it("includes incomplete latest DB snapshot status and issues", () => {
    const fixture = healthFixture()
    fixture.latestDbSnapshot = {
      path: "/tmp/hope/memory-repair-snapshots/20260707T100000Z",
      createdAt: "2026-07-07T10:00:00.000Z",
      status: "missing_files",
      issues: ["missing file: memory.db"],
      files: [],
    }

    const markdown = formatMemoryHealthDiagnostics(fixture)

    expect(markdown).toContain("- Status: missing_files")
    expect(markdown).toContain("- Issue: missing file: memory.db")
  })

  it("redacts provider diagnostic secrets in copied diagnostics", () => {
    const fixture = healthFixture()
    fixture.externalProviders[0] = {
      ...fixture.externalProviders[0],
      endpointConfigured: true,
      lastError:
        "request failed https://api.example.test/memory?token=tok-secret&safe=1 Authorization: Bearer bearer-secret api_key=sk-live-secret sk-testsecret123456",
    }

    const markdown = formatMemoryHealthDiagnostics(fixture)

    expect(markdown).toContain("token=[redacted]")
    expect(markdown).toContain("Authorization: Bearer [redacted]")
    expect(markdown).toContain("api_key=[redacted]")
    expect(markdown).toContain("sk-[redacted]")
    expect(markdown).not.toContain("tok-secret")
    expect(markdown).not.toContain("bearer-secret")
    expect(markdown).not.toContain("sk-live-secret")
    expect(markdown).not.toContain("sk-testsecret123456")
  })

  it("bounds multiline provider diagnostics after redaction", () => {
    const fixture = healthFixture()
    const longTail = "x".repeat(900)
    fixture.externalProviders[0] = {
      ...fixture.externalProviders[0],
      endpointConfigured: true,
      lastError: `first line\nsecond line token=secret-token\n${longTail}`,
    }

    const markdown = formatMemoryHealthDiagnostics(fixture)
    const providerLine = markdown
      .split("\n")
      .find((line) => line.startsWith("- Mem0 (mem0, mem0-1):"))

    expect(providerLine).toBeTruthy()
    expect(providerLine).toContain("token=[redacted]")
    expect(providerLine).toContain("[truncated]")
    expect(providerLine).not.toContain("secret-token")
    expect(providerLine).not.toContain("\nsecond line")
    expect(providerLine!.length).toBeLessThan(760)
  })

  it("sanitizes provider identity and issue text in copied diagnostics", () => {
    const fixture = healthFixture()
    fixture.externalProviders[0] = {
      ...fixture.externalProviders[0],
      id: `custom\nid token=id-secret ${"i".repeat(260)}`,
      displayName: `Custom\nProvider api_key=name-secret ${"n".repeat(260)}`,
      lastSyncAt: `2026-07-07T10:00:00Z\nAuthorization: Bearer sync-secret ${"s".repeat(260)}`,
      lastError: null,
    }
    fixture.issues = [
      {
        code: `external_provider\nwarning token=issue-code-secret ${"c".repeat(260)}`,
        severity: "warning",
        message: `provider failed\napi_key=issue-message-secret ${"m".repeat(900)}`,
        action: `rotate token=issue-action-secret\nthen retry`,
      },
    ]

    const markdown = formatMemoryHealthDiagnostics(fixture)

    expect(markdown).toContain("token=[redacted]")
    expect(markdown).toContain("api_key=[redacted]")
    expect(markdown).toContain("Authorization: Bearer [redacted]")
    expect(markdown).toContain("[truncated]")
    expect(markdown).not.toContain("id-secret")
    expect(markdown).not.toContain("name-secret")
    expect(markdown).not.toContain("sync-secret")
    expect(markdown).not.toContain("issue-code-secret")
    expect(markdown).not.toContain("issue-message-secret")
    expect(markdown).not.toContain("issue-action-secret")
    expect(markdown).not.toContain("Custom\nProvider")
    expect(markdown).not.toContain("provider failed\napi_key")
  })

  it("explains snapshot-first repair policy when database integrity fails", () => {
    const fixture = healthFixture()
    fixture.quickCheck = "database disk image is malformed"
    fixture.ftsMissingRows = 10

    const markdown = formatMemoryHealthDiagnostics(fixture)

    expect(markdown).toContain("- Repair policy: snapshot_first")
    expect(markdown).toContain(
      "- Policy note: SQLite integrity check failed; create a database snapshot before running other repairs.",
    )
    expect(markdown).toContain("- create_db_snapshot: Create database snapshot.")
    expect(markdown).not.toContain("- rebuild_fts: Rebuild keyword index.")
  })
})

describe("formatDeepResolverHealthSummary", () => {
  it("renders a clear state with localized LLM off label", () => {
    const summary = formatDeepResolverHealthSummary(
      {
        deepResolverExpiredCandidates: 0,
        deepResolverConflictGroups: 0,
        deepResolverGroupsToAnalyze: 0,
        deepResolverGroupCap: 50,
        deepResolverWouldCallLlm: false,
        deepResolverBlockingReasons: [],
      },
      t,
      { onLabel: "On", offLabel: "Off" },
    )

    expect(summary.tone).toBe("clear")
    expect(summary.backlogCount).toBe(0)
    expect(summary.statusText).toBe("Deep Resolver backlog is clear.")
    expect(summary.detailText).toBe("0/50 group(s) would be analyzed · LLM Off")
  })

  it("renders backlog counts and localized LLM on label", () => {
    const summary = formatDeepResolverHealthSummary(
      {
        deepResolverExpiredCandidates: 2,
        deepResolverConflictGroups: 3,
        deepResolverGroupsToAnalyze: 3,
        deepResolverGroupCap: 50,
        deepResolverWouldCallLlm: true,
        deepResolverBlockingReasons: [],
      },
      t,
      { onLabel: "开", offLabel: "关" },
    )

    expect(summary.tone).toBe("backlog")
    expect(summary.expiredCandidates).toBe(2)
    expect(summary.conflictGroups).toBe(3)
    expect(summary.backlogCount).toBe(5)
    expect(summary.statusText).toBe("Deep Resolver backlog: 2 expired · 3 conflict group(s)")
    expect(summary.detailText).toBe("3/50 group(s) would be analyzed · LLM 开")
  })

  it("renders blocking reasons without a runnable detail row", () => {
    const summary = formatDeepResolverHealthSummary(
      {
        deepResolverExpiredCandidates: 2,
        deepResolverConflictGroups: 3,
        deepResolverGroupsToAnalyze: 3,
        deepResolverGroupCap: 50,
        deepResolverWouldCallLlm: true,
        deepResolverBlockingReasons: [
          "dreaming_disabled",
          "claim_load_failed",
          "unknown_reason",
        ],
      },
      t,
      { onLabel: "On", offLabel: "Off" },
    )

    expect(summary.tone).toBe("blocked")
    expect(summary.backlogCount).toBe(5)
    expect(summary.statusText).toBe(
      "Deep Resolver blocked: Dreaming off, Claims unavailable, unknown reason",
    )
    expect(summary.detailText).toBeNull()
  })
})
