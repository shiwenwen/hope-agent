import { describe, expect, it } from "vitest"

import { spriteSettingsErrorDetail, spriteSettingsErrorToast } from "./spriteSettingsFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("sprite settings feedback", () => {
  it("extracts and redacts sprite settings diagnostics", () => {
    expect(spriteSettingsErrorDetail(new Error("config store locked"))).toBe("config store locked")
    expect(
      spriteSettingsErrorDetail(
        "sprite settings failed Authorization: Bearer sprite-secret token=query-secret api_key=side-secret",
      ),
    ).toBe(
      "sprite settings failed Authorization: Bearer [redacted] token=[redacted] api_key=[redacted]",
    )
    expect(spriteSettingsErrorDetail("  failed  ")).toBe("failed")
    expect(spriteSettingsErrorDetail("   ")).toBeNull()
    expect(spriteSettingsErrorDetail(undefined)).toBeNull()
  })

  it("formats localized sprite settings errors with redacted detail", () => {
    const translations: Record<string, string> = {
      "settings.sprite.errors.load": "加载精灵设置失败",
      "settings.sprite.errors.save": "保存精灵设置失败",
      "settings.sprite.errors.toggle": "更新精灵模式失败",
      "settings.sprite.errors.detail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(spriteSettingsErrorToast("load", t, "load token=load-secret")).toEqual({
      title: "加载精灵设置失败",
      description: "详细信息：load token=[redacted]",
    })
    expect(
      spriteSettingsErrorToast("toggle", t, "save Authorization: Bearer toggle-secret"),
    ).toEqual({
      title: "更新精灵模式失败",
      description: "详细信息：save Authorization: Bearer [redacted]",
    })
    expect(spriteSettingsErrorToast("save", t, null)).toEqual({
      title: "保存精灵设置失败",
    })
  })

  it("uses English fallbacks when translations are missing", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(spriteSettingsErrorToast("save", t, "denied")).toEqual({
      title: "Failed to save sprite settings",
      description: "Details: denied",
    })
  })
})
