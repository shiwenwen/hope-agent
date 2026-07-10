import { describe, expect, it } from "vitest"

import {
  profileSnapshotDiagnosticText,
  profileSnapshotOperationErrorDetail,
  profileSnapshotOperationErrorToast,
} from "./profileSnapshotOperationFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("profile snapshot operation feedback", () => {
  it("extracts user-facing error detail", () => {
    expect(profileSnapshotOperationErrorDetail(new Error("permission denied"))).toBe(
      "permission denied",
    )
    expect(profileSnapshotOperationErrorDetail("  source missing  ")).toBe("source missing")
    expect(profileSnapshotOperationErrorDetail("   ")).toBeNull()
    expect(profileSnapshotOperationErrorDetail(null)).toBeNull()
    expect(profileSnapshotOperationErrorDetail(undefined)).toBeNull()
    expect(
      profileSnapshotDiagnosticText(
        "profile failed https://api.example.test?token=query-secret Authorization: Bearer bearer-secret api_key=sk-live-secret",
      ),
    ).toBe(
      "profile failed https://api.example.test?token=[redacted] Authorization: Bearer [redacted] api_key=[redacted]",
    )
    expect(
      profileSnapshotOperationErrorDetail(
        "profile synthesis failed password=db-secret passphrase=backup-secret",
      ),
    ).toBe("profile synthesis failed password=[redacted] passphrase=[redacted]")
  })

  it("formats localized operation errors with redacted detail", () => {
    const translations: Record<string, string> = {
      "settings.profile.operationErrors.openEvidenceSource": "打开画像证据来源失败",
      "settings.profile.operationErrors.detail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(
      profileSnapshotOperationErrorToast("openEvidenceSource", t, "file not found"),
    ).toEqual({
      title: "打开画像证据来源失败",
      description: "详细信息：file not found",
    })
    expect(
      profileSnapshotOperationErrorToast(
        "openEvidenceSource",
        t,
        "file open failed token=profile-secret",
      ),
    ).toEqual({
      title: "打开画像证据来源失败",
      description: "详细信息：file open failed token=[redacted]",
    })
  })

  it("uses English fallback titles and omits empty detail", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(profileSnapshotOperationErrorToast("refresh", t, "dreaming disabled")).toEqual({
      title: "Failed to synthesise profile",
      description: "Details: dreaming disabled",
    })
    expect(profileSnapshotOperationErrorToast("openEvidenceSource", t, "   ")).toEqual({
      title: "Failed to open profile evidence source",
    })
  })
})
