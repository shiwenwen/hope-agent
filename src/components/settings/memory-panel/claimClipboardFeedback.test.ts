import { describe, expect, it } from "vitest"

import {
  claimClipboardDiagnosticText,
  claimClipboardErrorDetail,
  claimClipboardErrorToast,
} from "./claimClipboardFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("claim clipboard feedback", () => {
  it("extracts user-facing clipboard error detail", () => {
    expect(claimClipboardErrorDetail(new Error("clipboard denied"))).toBe("clipboard denied")
    expect(claimClipboardErrorDetail("  permission prompt dismissed  ")).toBe(
      "permission prompt dismissed",
    )
    expect(claimClipboardErrorDetail("   ")).toBeNull()
    expect(claimClipboardErrorDetail(null)).toBeNull()
    expect(claimClipboardErrorDetail(undefined)).toBeNull()
    expect(
      claimClipboardDiagnosticText(
        "copy evidence failed https://api.example.test?token=query-secret Authorization: Bearer bearer-secret api_key=sk-live-secret",
      ),
    ).toBe(
      "copy evidence failed https://api.example.test?token=[redacted] Authorization: Bearer [redacted] api_key=[redacted]",
    )
    expect(
      claimClipboardErrorDetail(
        "copy evidence failed password=db-secret passphrase=backup-secret",
      ),
    ).toBe("copy evidence failed password=[redacted] passphrase=[redacted]")
  })

  it("formats localized copy errors with redacted detail", () => {
    const translations: Record<string, string> = {
      "settings.claims.copyEvidenceFailed": "复制证据详情失败",
      "settings.claims.reviewHistoryExportFailed": "复制审核历史失败",
      "settings.claims.copyFailureDetail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(claimClipboardErrorToast("copyEvidence", t, "clipboard denied")).toEqual({
      title: "复制证据详情失败",
      description: "详细信息：clipboard denied",
    })
    expect(claimClipboardErrorToast("copyEvidence", t, "clipboard denied token=claim-secret")).toEqual(
      {
        title: "复制证据详情失败",
        description: "详细信息：clipboard denied token=[redacted]",
      },
    )
    expect(claimClipboardErrorToast("copyReviewHistoryExport", t, "quota exceeded")).toEqual({
      title: "复制审核历史失败",
      description: "详细信息：quota exceeded",
    })
  })

  it("uses English fallback titles and omits empty detail", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(claimClipboardErrorToast("copyLink", t, "blocked")).toEqual({
      title: "Failed to copy memory link",
      description: "Details: blocked",
    })
    expect(claimClipboardErrorToast("copyReviewHistoryItem", t, null)).toEqual({
      title: "Failed to copy review decision",
    })
  })
})
