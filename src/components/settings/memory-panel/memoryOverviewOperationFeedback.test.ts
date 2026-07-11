import { describe, expect, it } from "vitest"

import {
  memoryOverviewDiagnosticText,
  memoryOverviewInsightsIssue,
  memoryOverviewInsightsWarning,
  memoryOverviewLoadIssue,
  memoryOverviewLoadWarning,
  memoryOverviewOpenMemoryErrorToast,
  memoryOverviewPendingClaimsErrorToast,
  memoryOverviewOperationErrorDetail,
} from "./memoryOverviewOperationFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("memory overview operation feedback", () => {
  it("extracts user-facing error detail", () => {
    expect(memoryOverviewOperationErrorDetail(new Error("database is locked"))).toBe(
      "database is locked",
    )
    expect(memoryOverviewOperationErrorDetail("  IPC unavailable  ")).toBe("IPC unavailable")
    expect(
      memoryOverviewDiagnosticText(
        "overview failed https://api.example.test?token=query-secret Authorization: Bearer bearer-secret api_key=sk-live-secret",
      ),
    ).toBe(
      "overview failed https://api.example.test?token=[redacted] Authorization: Bearer [redacted] api_key=[redacted]",
    )
    expect(
      memoryOverviewOperationErrorDetail(
        "overview query failed password=db-secret passphrase=backup-secret",
      ),
    ).toBe("overview query failed password=[redacted] passphrase=[redacted]")
    expect(memoryOverviewOperationErrorDetail("   ")).toBeNull()
    expect(memoryOverviewOperationErrorDetail(null)).toBeNull()
    expect(memoryOverviewOperationErrorDetail(undefined)).toBeNull()
  })

  it("formats localized partial load warnings with source names and detail", () => {
    const translations: Record<string, string> = {
      "settings.memoryOverviewErrors.recentActivity": "部分最近记忆动态加载失败",
      "settings.memoryOverviewErrors.recentActivityDetail":
        "暂不可用：{{sources}}。详细信息：{{error}}",
      "settings.memoryOverviewErrors.sources.history": "记忆历史",
      "settings.memoryOverviewErrors.sources.dreamingRunDetail": "Dreaming 决策",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(
      memoryOverviewLoadWarning(
        [
          memoryOverviewLoadIssue("history", "database token=history-secret"),
          memoryOverviewLoadIssue("dreamingRunDetail", "detail skipped token=run-secret"),
          memoryOverviewLoadIssue("history", "duplicate source"),
        ],
        t,
      ),
    ).toEqual({
      title: "部分最近记忆动态加载失败",
      description: "暂不可用：记忆历史, Dreaming 决策。详细信息：database token=[redacted]",
    })
  })

  it("uses English fallback labels and omits empty detail", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(
      memoryOverviewLoadWarning(
        [
          memoryOverviewLoadIssue("memories", "   "),
          memoryOverviewLoadIssue("experienceHistory", null),
        ],
        t,
      ),
    ).toEqual({
      title: "Some recent memory activity could not load",
      description: "Unavailable: latest memories, experience history.",
    })
  })

  it("formats pending review queue errors with action context", () => {
    const translations: Record<string, string> = {
      "settings.memoryOverviewErrors.pendingClaims": "加载待审核队列失败",
      "settings.memoryOverviewErrors.pendingClaimsDetail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(memoryOverviewPendingClaimsErrorToast(t, "claim store locked")).toEqual({
      title: "加载待审核队列失败",
      description: "详细信息：claim store locked",
    })
    expect(memoryOverviewPendingClaimsErrorToast(t, "claim store token=claim-secret")).toEqual({
      title: "加载待审核队列失败",
      description: "详细信息：claim store token=[redacted]",
    })

    const fallbackT = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)
    expect(memoryOverviewPendingClaimsErrorToast(fallbackT, "   ")).toEqual({
      title: "Failed to load review queue",
    })
  })

  it("formats memory insight partial load warnings", () => {
    const translations: Record<string, string> = {
      "settings.memoryOverviewErrors.insights": "部分记忆洞察加载失败",
      "settings.memoryOverviewErrors.insightsDetail":
        "暂不可用：{{sources}}。详细信息：{{error}}",
      "settings.memoryOverviewErrors.insightsSourcesMap.profileClaims": "个人画像记忆",
      "settings.memoryOverviewErrors.insightsSourcesMap.profileSnapshots": "画像快照",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(
      memoryOverviewInsightsWarning(
        [
          memoryOverviewInsightsIssue("profileClaims", "claim query failed api_key=profile-secret"),
          memoryOverviewInsightsIssue("profileSnapshots", "snapshot query skipped"),
          memoryOverviewInsightsIssue("profileClaims", "duplicate"),
        ],
        t,
      ),
    ).toEqual({
      title: "部分记忆洞察加载失败",
      description:
        "暂不可用：个人画像记忆, 画像快照。详细信息：claim query failed api_key=[redacted]",
    })
  })

  it("formats memory activity open errors with action context", () => {
    const translations: Record<string, string> = {
      "settings.memoryOverviewErrors.openMemory": "打开记忆动态失败",
      "settings.memoryOverviewErrors.openMemoryDetail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(memoryOverviewOpenMemoryErrorToast(t, new Error("memory row missing"))).toEqual({
      title: "打开记忆动态失败",
      description: "详细信息：memory row missing",
    })
    expect(memoryOverviewOpenMemoryErrorToast(t, "memory row token=memory-secret")).toEqual({
      title: "打开记忆动态失败",
      description: "详细信息：memory row token=[redacted]",
    })

    const fallbackT = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)
    expect(memoryOverviewOpenMemoryErrorToast(fallbackT, null)).toEqual({
      title: "Failed to open memory activity",
    })
  })
})
