import { describe, expect, it } from "vitest"

import { knowledgeSpriteErrorDetail, knowledgeSpriteErrorToast } from "./knowledgeSpriteFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("knowledge sprite feedback", () => {
  it("extracts and redacts sprite diagnostics", () => {
    expect(knowledgeSpriteErrorDetail(new Error("config store locked"))).toBe(
      "config store locked",
    )
    expect(
      knowledgeSpriteErrorDetail(
        "sprite failed Authorization: Bearer sprite-secret token=query-secret api_key=side-secret",
      ),
    ).toBe(
      "sprite failed Authorization: Bearer [redacted] token=[redacted] api_key=[redacted]",
    )
    expect(knowledgeSpriteErrorDetail("  failed  ")).toBe("failed")
    expect(knowledgeSpriteErrorDetail("   ")).toBeNull()
    expect(knowledgeSpriteErrorDetail(undefined)).toBeNull()
  })

  it("formats localized sprite errors with redacted detail", () => {
    const translations: Record<string, string> = {
      "knowledge.sprite.loadFailed": "无法加载精灵模式",
      "knowledge.sprite.toggleFailed": "无法更新精灵模式",
      "knowledge.sprite.observeFailed": "无法让精灵生成建议",
      "knowledge.sprite.errorDetail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(knowledgeSpriteErrorToast("loadConfig", t, "load token=load-secret")).toEqual({
      title: "无法加载精灵模式",
      description: "详细信息：load token=[redacted]",
    })
    expect(
      knowledgeSpriteErrorToast("saveToggle", t, "save Authorization: Bearer toggle-secret"),
    ).toEqual({
      title: "无法更新精灵模式",
      description: "详细信息：save Authorization: Bearer [redacted]",
    })
    expect(knowledgeSpriteErrorToast("observe", t, null)).toEqual({
      title: "无法让精灵生成建议",
    })
  })

  it("uses English fallbacks when translations are missing", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(knowledgeSpriteErrorToast("observe", t, "denied")).toEqual({
      title: "Couldn't ask the sprite for a suggestion",
      description: "Details: denied",
    })
  })
})
