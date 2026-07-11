import { describe, expect, it } from "vitest"

import {
  workspaceSourceDiagnosticText,
  workspaceSourceErrorDetail,
  workspaceSourceOpenErrorToast,
} from "./workspaceSourceFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("workspace source feedback", () => {
  it("formats source open failures with redacted detail", () => {
    expect(workspaceSourceErrorDetail(new Error("popup blocked"))).toBe("popup blocked")
    expect(workspaceSourceErrorDetail("   ")).toBeNull()
    expect(
      workspaceSourceDiagnosticText(
        "open failed https://api.example.test?token=query-secret Authorization: Bearer bearer-secret api_key=sk-live-secret",
      ),
    ).toBe(
      "open failed https://api.example.test?token=[redacted] Authorization: Bearer [redacted] api_key=[redacted]",
    )

    const translations: Record<string, string> = {
      "workspace.openSourceFailed": "打开来源失败",
      "workspace.openSourceDetail": "详细信息：{{error}}",
    }
    const t = (key: string, defaultValue: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? defaultValue, options)

    expect(workspaceSourceOpenErrorToast(t, "popup token=workspace-secret")).toEqual({
      title: "打开来源失败",
      description: "详细信息：popup token=[redacted]",
    })
    expect(workspaceSourceOpenErrorToast(t, null)).toEqual({
      title: "打开来源失败",
    })
  })

  it("uses English fallback titles", () => {
    const t = (_key: string, defaultValue: string, options?: Record<string, unknown>) =>
      interpolate(defaultValue, options)

    expect(workspaceSourceOpenErrorToast(t, "browser denied")).toEqual({
      title: "Failed to open source",
      description: "Details: browser denied",
    })
  })
})
