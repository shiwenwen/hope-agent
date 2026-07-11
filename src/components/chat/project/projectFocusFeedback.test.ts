import { describe, expect, it } from "vitest"

import {
  projectFocusErrorDetail,
  projectFocusLoadErrorToast,
  projectFocusMissingToast,
} from "./projectFocusFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("project focus feedback", () => {
  it("extracts and redacts project focus error detail", () => {
    expect(projectFocusErrorDetail(new Error("project list failed"))).toBe("project list failed")
    expect(
      projectFocusErrorDetail(
        "project query failed Authorization: Bearer bearer-secret api_key=project-secret",
      ),
    ).toBe(
      "project query failed Authorization: Bearer [redacted] api_key=[redacted]",
    )
    expect(projectFocusErrorDetail("   ")).toBeNull()
    expect(projectFocusErrorDetail(null)).toBeNull()
  })

  it("formats localized project load failures", () => {
    const translations: Record<string, string> = {
      "project.openFromMemoryLoadFailed": "打开项目来源失败",
      "project.openFromMemoryLoadFailedDetail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(projectFocusLoadErrorToast(t, "database token=project-secret")).toEqual({
      title: "打开项目来源失败",
      description: "详细信息：database token=[redacted]",
    })
  })

  it("formats missing project fallback", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(projectFocusMissingToast(t)).toEqual({
      title: "Project is no longer available",
      description: "The memory source points to a project that was deleted or cannot be found.",
    })
  })
})
