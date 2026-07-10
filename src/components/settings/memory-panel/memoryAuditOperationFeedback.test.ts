import { describe, expect, it } from "vitest"

import {
  memoryAuditDiagnosticText,
  memoryAuditDegradedIssue,
  memoryAuditDegradedWarning,
  memoryAuditOperationErrorDetail,
  memoryAuditOperationErrorToast,
} from "./memoryAuditOperationFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("memory audit operation feedback", () => {
  it("extracts user-facing error detail", () => {
    expect(memoryAuditOperationErrorDetail(new Error("clipboard denied"))).toBe(
      "clipboard denied",
    )
    expect(memoryAuditOperationErrorDetail("  database is locked  ")).toBe("database is locked")
    expect(memoryAuditOperationErrorDetail("   ")).toBeNull()
    expect(memoryAuditOperationErrorDetail(null)).toBeNull()
    expect(memoryAuditOperationErrorDetail(undefined)).toBeNull()
    expect(
      memoryAuditDiagnosticText(
        "audit failed https://api.example.test?token=query-secret Authorization: Bearer bearer-secret api_key=sk-live-secret",
      ),
    ).toBe(
      "audit failed https://api.example.test?token=[redacted] Authorization: Bearer [redacted] api_key=[redacted]",
    )
    expect(
      memoryAuditOperationErrorDetail(
        "audit export failed password=db-secret passphrase=backup-secret",
      ),
    ).toBe("audit export failed password=[redacted] passphrase=[redacted]")
  })

  it("formats localized operation errors with redacted detail", () => {
    const translations: Record<string, string> = {
      "settings.memoryAuditErrors.loadMore": "加载更多记忆动态失败",
      "settings.memoryAuditErrors.detail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(memoryAuditOperationErrorToast("loadMore", t, "timeout")).toEqual({
      title: "加载更多记忆动态失败",
      description: "详细信息：timeout",
    })
    expect(memoryAuditOperationErrorToast("loadMore", t, "timeout token=audit-secret")).toEqual({
      title: "加载更多记忆动态失败",
      description: "详细信息：timeout token=[redacted]",
    })
  })

  it("uses English fallback titles and omits empty detail", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(memoryAuditOperationErrorToast("exportAll", t, "permission denied")).toEqual({
      title: "Failed to copy all matching memory activity",
      description: "Details: permission denied",
    })
    expect(memoryAuditOperationErrorToast("exportCurrent", t, "   ")).toEqual({
      title: "Failed to copy current memory activity",
    })
    expect(memoryAuditOperationErrorToast("search", t, null)).toEqual({
      title: "Failed to search memory activity",
    })
  })

  it("formats degraded audit warnings with redacted detail", () => {
    const translations: Record<string, string> = {
      "settings.memoryAuditWarnings.degraded": "记忆动态结果可能不完整",
      "settings.memoryAuditWarnings.degradedDetail":
        "已降级查询：{{sources}}。详细信息：{{error}}",
      "settings.memoryAuditWarnings.sources.unified": "统一审计查询",
      "settings.memoryAuditWarnings.sources.decisions": "结构化记忆决策",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(
      memoryAuditDegradedWarning(
        [
          memoryAuditDegradedIssue("unified", "aggregate unavailable token=aggregate-secret"),
          memoryAuditDegradedIssue("decisions", "decision query failed"),
          memoryAuditDegradedIssue("unified", "duplicate source"),
        ],
        t,
      ),
    ).toEqual({
      title: "记忆动态结果可能不完整",
      description:
        "已降级查询：统一审计查询, 结构化记忆决策。详细信息：aggregate unavailable token=[redacted]",
    })

    const fallbackT = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)
    expect(
      memoryAuditDegradedWarning([memoryAuditDegradedIssue("experience", "   ")], fallbackT),
    ).toEqual({
      title: "Memory activity results may be incomplete",
      description: "Fallback used for: workflow history.",
    })
  })
})
