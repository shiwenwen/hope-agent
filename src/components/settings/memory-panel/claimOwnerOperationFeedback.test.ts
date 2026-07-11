import { describe, expect, it } from "vitest"

import {
  claimOwnerDiagnosticText,
  claimOwnerOperationErrorDetail,
  claimOwnerOperationErrorToast,
} from "./claimOwnerOperationFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("claim owner operation feedback", () => {
  it("extracts user-facing error detail", () => {
    expect(claimOwnerOperationErrorDetail(new Error("claim is locked"))).toBe("claim is locked")
    expect(claimOwnerOperationErrorDetail("  stale evidence  ")).toBe("stale evidence")
    expect(
      claimOwnerDiagnosticText(
        "claim update failed https://api.example.test?token=query-secret Authorization: Bearer bearer-secret api_key=sk-live-secret",
      ),
    ).toBe(
      "claim update failed https://api.example.test?token=[redacted] Authorization: Bearer [redacted] api_key=[redacted]",
    )
    expect(
      claimOwnerOperationErrorDetail(
        "claim graph failed password=db-secret passphrase=backup-secret",
      ),
    ).toBe("claim graph failed password=[redacted] passphrase=[redacted]")
    expect(claimOwnerOperationErrorDetail("   ")).toBeNull()
    expect(claimOwnerOperationErrorDetail(null)).toBeNull()
    expect(claimOwnerOperationErrorDetail(undefined)).toBeNull()
  })

  it("formats localized operation errors while redacting sensitive detail", () => {
    const translations: Record<string, string> = {
      "settings.claims.operationErrors.conflictResolve": "处理记忆冲突失败",
      "settings.claims.operationErrors.loadSchema": "加载结构化记忆筛选项失败",
      "settings.claims.operationErrors.loadList": "加载结构化记忆列表失败",
      "settings.claims.operationErrors.loadScopeNames": "加载记忆作用域名称失败",
      "settings.claims.operationErrors.loadListSummaries": "结构化记忆列表详情可能不完整",
      "settings.claims.operationErrors.loadReviewHistory": "加载纠错历史失败",
      "settings.claims.operationErrors.loadDetail": "打开结构化记忆失败",
      "settings.claims.operationErrors.openEvidenceSource": "打开证据来源失败",
      "settings.claims.operationErrors.detail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(claimOwnerOperationErrorToast("conflictResolve", t, "database is locked")).toEqual({
      title: "处理记忆冲突失败",
      description: "详细信息：database is locked",
    })
    expect(
      claimOwnerOperationErrorToast(
        "loadSchema",
        t,
        "schema metadata failed Authorization: Bearer schema-secret",
      ),
    ).toEqual({
      title: "加载结构化记忆筛选项失败",
      description: "详细信息：schema metadata failed Authorization: Bearer [redacted]",
    })
    expect(claimOwnerOperationErrorToast("loadList", t, "list token=list-secret")).toEqual({
      title: "加载结构化记忆列表失败",
      description: "详细信息：list token=[redacted]",
    })
    expect(
      claimOwnerOperationErrorToast("loadScopeNames", t, "project api_key=scope-secret"),
    ).toEqual({
      title: "加载记忆作用域名称失败",
      description: "详细信息：project api_key=[redacted]",
    })
    expect(
      claimOwnerOperationErrorToast("loadListSummaries", t, "summary api_key=summary-secret"),
    ).toEqual({
      title: "结构化记忆列表详情可能不完整",
      description: "详细信息：summary api_key=[redacted]",
    })
    expect(
      claimOwnerOperationErrorToast("loadReviewHistory", t, "history token=history-secret"),
    ).toEqual({
      title: "加载纠错历史失败",
      description: "详细信息：history token=[redacted]",
    })
    expect(claimOwnerOperationErrorToast("loadDetail", t, "claim row missing")).toEqual({
      title: "打开结构化记忆失败",
      description: "详细信息：claim row missing",
    })
    expect(claimOwnerOperationErrorToast("openEvidenceSource", t, "file not found")).toEqual({
      title: "打开证据来源失败",
      description: "详细信息：file not found",
    })
    expect(
      claimOwnerOperationErrorToast("loadDetail", t, "claim_get failed token=claim-secret"),
    ).toEqual({
      title: "打开结构化记忆失败",
      description: "详细信息：claim_get failed token=[redacted]",
    })
  })

  it("uses English fallback titles and omits empty detail", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(claimOwnerOperationErrorToast("backfillPlan", t, "provider unavailable")).toEqual({
      title: "Failed to compute the backfill plan",
      description: "Details: provider unavailable",
    })
    expect(claimOwnerOperationErrorToast("loadSchema", t, "claim schema unavailable")).toEqual({
      title: "Failed to load structured memory filters",
      description: "Details: claim schema unavailable",
    })
    expect(claimOwnerOperationErrorToast("loadList", t, "claim_list unavailable")).toEqual({
      title: "Failed to load structured memory list",
      description: "Details: claim_list unavailable",
    })
    expect(claimOwnerOperationErrorToast("loadScopeNames", t, "project list failed")).toEqual({
      title: "Failed to load memory scope names",
      description: "Details: project list failed",
    })
    expect(claimOwnerOperationErrorToast("loadListSummaries", t, "summary timeout")).toEqual({
      title: "Structured memory list details may be incomplete",
      description: "Details: summary timeout",
    })
    expect(claimOwnerOperationErrorToast("loadReviewHistory", t, "history timeout")).toEqual({
      title: "Failed to load review history",
      description: "Details: history timeout",
    })
    expect(claimOwnerOperationErrorToast("loadDetail", t, "database is locked")).toEqual({
      title: "Failed to open structured memory",
      description: "Details: database is locked",
    })
    expect(claimOwnerOperationErrorToast("restoreArchive", t, "   ")).toEqual({
      title: "Failed to restore archived memory",
    })
    expect(
      claimOwnerOperationErrorToast("batchAction", t, new Error("permission denied")),
    ).toEqual({
      title: "Batch action failed",
      description: "Details: permission denied",
    })
    expect(claimOwnerOperationErrorToast("openEvidenceSource", t, null)).toEqual({
      title: "Failed to open evidence source",
    })
  })
})
