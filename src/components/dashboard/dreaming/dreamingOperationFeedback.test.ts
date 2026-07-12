import { describe, expect, it } from "vitest"

import {
  dreamingDiagnosticText,
  dreamingOperationErrorDetail,
  dreamingOperationErrorToast,
} from "./dreamingOperationFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("dreaming operation feedback", () => {
  it("extracts user-facing error detail", () => {
    expect(dreamingOperationErrorDetail(new Error("sqlite is locked"))).toBe("sqlite is locked")
    expect(dreamingOperationErrorDetail("  run missing  ")).toBe("run missing")
    expect(dreamingOperationErrorDetail("   ")).toBeNull()
    expect(dreamingOperationErrorDetail(null)).toBeNull()
    expect(dreamingOperationErrorDetail(undefined)).toBeNull()
    expect(
      dreamingDiagnosticText(
        "dreaming run failed https://api.example.test?token=query-secret Authorization: Bearer bearer-secret api_key=sk-live-secret",
      ),
    ).toBe(
      "dreaming run failed https://api.example.test?token=[redacted] Authorization: Bearer [redacted] api_key=[redacted]",
    )
    expect(
      dreamingOperationErrorDetail(
        "dreaming resolver failed password=db-secret passphrase=backup-secret",
      ),
    ).toBe("dreaming resolver failed password=[redacted] passphrase=[redacted]")
  })

  it("formats localized operation errors with redacted detail", () => {
    const translations: Record<string, string> = {
      "dashboard.dreaming.errors.loadRuns": "加载 Dreaming 运行历史失败",
      "dashboard.dreaming.errors.loadEvidenceQuote": "加载证据摘录失败",
      "dashboard.dreaming.errors.resolverPreflight": "加载深度整理预检失败",
      "dashboard.dreaming.errors.detail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(dreamingOperationErrorToast("loadRuns", t, "query failed")).toEqual({
      title: "加载 Dreaming 运行历史失败",
      description: "详细信息：query failed",
    })
    expect(dreamingOperationErrorToast("loadRuns", t, "query failed token=dreaming-secret")).toEqual(
      {
        title: "加载 Dreaming 运行历史失败",
        description: "详细信息：query failed token=[redacted]",
      },
    )
    expect(dreamingOperationErrorToast("loadEvidenceQuote", t, "permission denied")).toEqual({
      title: "加载证据摘录失败",
      description: "详细信息：permission denied",
    })
    expect(dreamingOperationErrorToast("resolverPreflight", t, "database is locked")).toEqual({
      title: "加载深度整理预检失败",
      description: "详细信息：database is locked",
    })
  })

  it("uses English fallback titles and omits empty detail", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(dreamingOperationErrorToast("loadDiary", t, "file missing")).toEqual({
      title: "Failed to open Dreaming diary",
      description: "Details: file missing",
    })
    expect(dreamingOperationErrorToast("runNow", t, "   ")).toEqual({
      title: "Failed to run Dreaming",
    })
    expect(dreamingOperationErrorToast("runResolver", t, null)).toEqual({
      title: "Deep resolve failed",
    })
    expect(dreamingOperationErrorToast("resolverPreflight", t, "db unavailable")).toEqual({
      title: "Failed to load Deep Resolver preflight",
      description: "Details: db unavailable",
    })
  })
})
