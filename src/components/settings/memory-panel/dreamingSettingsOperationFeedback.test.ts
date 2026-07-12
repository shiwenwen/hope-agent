import { describe, expect, it } from "vitest"

import {
  dreamingSettingsDiagnosticText,
  dreamingSettingsOperationErrorDetail,
  dreamingSettingsOperationErrorToast,
} from "./dreamingSettingsOperationFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("dreaming settings operation feedback", () => {
  it("extracts user-facing error detail", () => {
    expect(dreamingSettingsOperationErrorDetail(new Error("config file denied"))).toBe(
      "config file denied",
    )
    expect(dreamingSettingsOperationErrorDetail("  invalid cron  ")).toBe("invalid cron")
    expect(dreamingSettingsOperationErrorDetail("   ")).toBeNull()
    expect(dreamingSettingsOperationErrorDetail(null)).toBeNull()
    expect(dreamingSettingsOperationErrorDetail(undefined)).toBeNull()
    expect(
      dreamingSettingsDiagnosticText(
        "dreaming settings failed https://api.example.test?token=query-secret Authorization: Bearer bearer-secret api_key=sk-live-secret",
      ),
    ).toBe(
      "dreaming settings failed https://api.example.test?token=[redacted] Authorization: Bearer [redacted] api_key=[redacted]",
    )
    expect(
      dreamingSettingsOperationErrorDetail(
        "dreaming config failed password=db-secret passphrase=backup-secret",
      ),
    ).toBe("dreaming config failed password=[redacted] passphrase=[redacted]")
  })

  it("formats localized operation errors with redacted detail", () => {
    const translations: Record<string, string> = {
      "settings.dreaming.errors.loadConfig": "加载 Dreaming 设置失败",
      "settings.dreaming.errors.loadStatus": "加载 Dreaming 状态失败",
      "settings.dreaming.errors.reloadAfterSave": "保存失败后重新加载 Dreaming 设置失败",
      "settings.dreaming.errors.saveConfig": "保存 Dreaming 设置失败",
      "settings.dreaming.errors.detail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(dreamingSettingsOperationErrorToast("loadConfig", t, "config read failed")).toEqual({
      title: "加载 Dreaming 设置失败",
      description: "详细信息：config read failed",
    })
    expect(
      dreamingSettingsOperationErrorToast(
        "loadConfig",
        t,
        "config read failed token=dreaming-settings-secret",
      ),
    ).toEqual({
      title: "加载 Dreaming 设置失败",
      description: "详细信息：config read failed token=[redacted]",
    })
    expect(dreamingSettingsOperationErrorToast("loadStatus", t, "status query failed")).toEqual({
      title: "加载 Dreaming 状态失败",
      description: "详细信息：status query failed",
    })
    expect(dreamingSettingsOperationErrorToast("saveConfig", t, "permission denied")).toEqual({
      title: "保存 Dreaming 设置失败",
      description: "详细信息：permission denied",
    })
    expect(
      dreamingSettingsOperationErrorToast("reloadAfterSave", t, "api_key=reload-secret"),
    ).toEqual({
      title: "保存失败后重新加载 Dreaming 设置失败",
      description: "详细信息：api_key=[redacted]",
    })
  })

  it("uses English fallback titles and omits empty detail", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(dreamingSettingsOperationErrorToast("loadConfig", t, "   ")).toEqual({
      title: "Failed to load Dreaming settings",
    })
    expect(dreamingSettingsOperationErrorToast("loadModels", t, "provider list failed")).toEqual({
      title: "Failed to load Dreaming model list",
      description: "Details: provider list failed",
    })
    expect(dreamingSettingsOperationErrorToast("saveConfig", t, "database is locked")).toEqual({
      title: "Failed to save Dreaming settings",
      description: "Details: database is locked",
    })
    expect(
      dreamingSettingsOperationErrorToast("reloadAfterSave", t, "config backend unavailable"),
    ).toEqual({
      title: "Failed to reload Dreaming settings after save failed",
      description: "Details: config backend unavailable",
    })
  })
})
