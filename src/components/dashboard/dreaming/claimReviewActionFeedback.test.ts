import { describe, expect, it } from "vitest"

import {
  claimReviewActionDiagnosticText,
  claimReviewActionErrorDetail,
  claimReviewActionErrorToast,
} from "./claimReviewActionFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("claim review action feedback", () => {
  it("extracts user-facing error detail", () => {
    expect(claimReviewActionErrorDetail(new Error("claim row locked"))).toBe("claim row locked")
    expect(claimReviewActionErrorDetail("  stale claim  ")).toBe("stale claim")
    expect(claimReviewActionErrorDetail("   ")).toBeNull()
    expect(claimReviewActionErrorDetail(null)).toBeNull()
    expect(claimReviewActionErrorDetail(undefined)).toBeNull()
    expect(
      claimReviewActionDiagnosticText(
        "review action failed https://api.example.test?token=query-secret Authorization: Bearer bearer-secret api_key=sk-live-secret",
      ),
    ).toBe(
      "review action failed https://api.example.test?token=[redacted] Authorization: Bearer [redacted] api_key=[redacted]",
    )
    expect(
      claimReviewActionErrorDetail(
        "review action failed password=db-secret passphrase=backup-secret",
      ),
    ).toBe("review action failed password=[redacted] passphrase=[redacted]")
  })

  it("formats localized operation errors with redacted detail", () => {
    const translations: Record<string, string> = {
      "dashboard.dreaming.review.errors.loadQueue": "加载待审核队列失败",
      "dashboard.dreaming.review.errors.edit": "编辑记忆失败",
      "dashboard.dreaming.review.errors.forget": "忘记记忆失败",
      "dashboard.dreaming.review.errors.detail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(claimReviewActionErrorToast("loadQueue", t, "database is locked")).toEqual({
      title: "加载待审核队列失败",
      description: "详细信息：database is locked",
    })
    expect(
      claimReviewActionErrorToast("loadQueue", t, "database is locked token=review-secret"),
    ).toEqual({
      title: "加载待审核队列失败",
      description: "详细信息：database is locked token=[redacted]",
    })
    expect(claimReviewActionErrorToast("edit", t, "permission denied")).toEqual({
      title: "编辑记忆失败",
      description: "详细信息：permission denied",
    })
    expect(claimReviewActionErrorToast("forget", t, "orphan link failed")).toEqual({
      title: "忘记记忆失败",
      description: "详细信息：orphan link failed",
    })
  })

  it("uses English fallback titles and omits empty detail", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(claimReviewActionErrorToast("approve", t, "claim missing")).toEqual({
      title: "Failed to approve memory",
      description: "Details: claim missing",
    })
    expect(claimReviewActionErrorToast("moveScope", t, "   ")).toEqual({
      title: "Failed to move memory scope",
    })
    expect(claimReviewActionErrorToast("reject", t, null)).toEqual({
      title: "Failed to reject memory",
    })
  })
})
