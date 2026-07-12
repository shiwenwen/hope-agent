import { describe, expect, it } from "vitest"

import {
  memoryExperienceDiagnosticText,
  memoryExperienceOperationErrorDetail,
  memoryExperienceOperationErrorToast,
} from "./memoryExperienceOperationFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("memory experience operation feedback", () => {
  it("extracts user-facing error detail", () => {
    expect(memoryExperienceOperationErrorDetail(new Error("procedure missing"))).toBe(
      "procedure missing",
    )
    expect(memoryExperienceOperationErrorDetail("  index unavailable  ")).toBe(
      "index unavailable",
    )
    expect(memoryExperienceOperationErrorDetail("   ")).toBeNull()
    expect(memoryExperienceOperationErrorDetail(null)).toBeNull()
    expect(memoryExperienceOperationErrorDetail(undefined)).toBeNull()
    expect(
      memoryExperienceDiagnosticText(
        "workflow failed https://api.example.test?token=query-secret Authorization: Bearer bearer-secret api_key=sk-live-secret",
      ),
    ).toBe(
      "workflow failed https://api.example.test?token=[redacted] Authorization: Bearer [redacted] api_key=[redacted]",
    )
    expect(
      memoryExperienceOperationErrorDetail(
        "procedure write failed password=db-secret passphrase=backup-secret",
      ),
    ).toBe("procedure write failed password=[redacted] passphrase=[redacted]")
  })

  it("formats localized operation errors with redacted detail", () => {
    const translations: Record<string, string> = {
      "settings.memoryExperienceErrors.saveProcedure": "保存流程失败",
      "settings.memoryExperienceErrors.detail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(memoryExperienceOperationErrorToast("saveProcedure", t, "database is locked")).toEqual({
      title: "保存流程失败",
      description: "详细信息：database is locked",
    })
    expect(
      memoryExperienceOperationErrorToast("saveProcedure", t, "database token=experience-secret"),
    ).toEqual({
      title: "保存流程失败",
      description: "详细信息：database token=[redacted]",
    })
  })

  it("uses English fallback titles and omits empty detail", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(memoryExperienceOperationErrorToast("promoteEpisode", t, "LLM disabled")).toEqual({
      title: "Failed to promote episode",
      description: "Details: LLM disabled",
    })
    expect(memoryExperienceOperationErrorToast("openExperience", t, "   ")).toEqual({
      title: "Failed to open experience memory",
    })
    expect(memoryExperienceOperationErrorToast("focusExperience", t, "IPC denied")).toEqual({
      title: "Failed to focus experience memory",
      description: "Details: IPC denied",
    })
    expect(memoryExperienceOperationErrorToast("loadHistory", t, "history table locked")).toEqual({
      title: "Failed to load experience history",
      description: "Details: history table locked",
    })
    expect(
      memoryExperienceOperationErrorToast("restoreExperience", t, new Error("permission denied")),
    ).toEqual({
      title: "Failed to restore experience",
      description: "Details: permission denied",
    })
  })
})
