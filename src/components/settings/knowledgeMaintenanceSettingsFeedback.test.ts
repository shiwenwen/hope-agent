import { describe, expect, it } from "vitest"

import {
  knowledgeMaintenanceSettingsErrorDetail,
  knowledgeMaintenanceSettingsErrorToast,
} from "./knowledgeMaintenanceSettingsFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("knowledge maintenance settings feedback", () => {
  it("extracts and redacts maintenance diagnostics", () => {
    expect(knowledgeMaintenanceSettingsErrorDetail(new Error("db busy"))).toBe("db busy")
    expect(
      knowledgeMaintenanceSettingsErrorDetail(
        "maintenance failed Authorization: Bearer maint-secret token=query-secret api_key=side-secret",
      ),
    ).toBe(
      "maintenance failed Authorization: Bearer [redacted] token=[redacted] api_key=[redacted]",
    )
    expect(knowledgeMaintenanceSettingsErrorDetail("  failed  ")).toBe("failed")
    expect(knowledgeMaintenanceSettingsErrorDetail("   ")).toBeNull()
    expect(knowledgeMaintenanceSettingsErrorDetail(undefined)).toBeNull()
  })

  it("formats localized maintenance setting errors with redacted detail", () => {
    const translations: Record<string, string> = {
      "settings.knowledgeMaintenance.errors.load": "加载自主维护设置失败",
      "settings.knowledgeMaintenance.errors.save": "保存自主维护设置失败",
      "settings.knowledgeMaintenance.errors.runNow": "运行自主维护失败",
      "settings.knowledgeMaintenance.errors.detail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(
      knowledgeMaintenanceSettingsErrorToast("load", t, "load token=load-secret"),
    ).toEqual({
      title: "加载自主维护设置失败",
      description: "详细信息：load token=[redacted]",
    })
    expect(
      knowledgeMaintenanceSettingsErrorToast(
        "runNow",
        t,
        "run Authorization: Bearer run-secret",
      ),
    ).toEqual({
      title: "运行自主维护失败",
      description: "详细信息：run Authorization: Bearer [redacted]",
    })
    expect(knowledgeMaintenanceSettingsErrorToast("save", t, null)).toEqual({
      title: "保存自主维护设置失败",
    })
  })

  it("uses English fallbacks when translations are missing", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(knowledgeMaintenanceSettingsErrorToast("save", t, "denied")).toEqual({
      title: "Failed to save autonomous maintenance settings",
      description: "Details: denied",
    })
  })
})
