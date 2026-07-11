import { describe, expect, it } from "vitest"

import {
  memoryAdvancedConfigDiagnosticText,
  memoryAdvancedConfigOperationErrorDetail,
  memoryAdvancedConfigOperationErrorToast,
} from "./memoryAdvancedConfigFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("memory advanced config feedback", () => {
  it("extracts user-facing error detail", () => {
    expect(memoryAdvancedConfigOperationErrorDetail(new Error("config write failed"))).toBe(
      "config write failed",
    )
    expect(memoryAdvancedConfigOperationErrorDetail("  invalid lambda  ")).toBe("invalid lambda")
    expect(memoryAdvancedConfigOperationErrorDetail("   ")).toBeNull()
    expect(memoryAdvancedConfigOperationErrorDetail(null)).toBeNull()
    expect(memoryAdvancedConfigOperationErrorDetail(undefined)).toBeNull()
    expect(
      memoryAdvancedConfigDiagnosticText(
        "advanced config failed https://api.example.test?token=query-secret Authorization: Bearer bearer-secret api_key=sk-live-secret",
      ),
    ).toBe(
      "advanced config failed https://api.example.test?token=[redacted] Authorization: Bearer [redacted] api_key=[redacted]",
    )
    expect(
      memoryAdvancedConfigOperationErrorDetail(
        "advanced config failed password=db-secret passphrase=backup-secret",
      ),
    ).toBe("advanced config failed password=[redacted] passphrase=[redacted]")
  })

  it("formats localized operation errors with redacted detail", () => {
    const translations: Record<string, string> = {
      "settings.memoryAdvancedConfigErrors.load": "加载记忆搜索调优失败",
      "settings.memoryAdvancedConfigErrors.saveHybrid": "保存混合搜索调优失败",
      "settings.memoryAdvancedConfigErrors.saveSelection": "保存 LLM 记忆选择调优失败",
      "settings.memoryAdvancedConfigErrors.saveTemporal": "保存时间衰减调优失败",
      "settings.memoryAdvancedConfigErrors.detail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(memoryAdvancedConfigOperationErrorToast("load", t, "config read failed")).toEqual({
      title: "加载记忆搜索调优失败",
      description: "详细信息：config read failed",
    })
    expect(
      memoryAdvancedConfigOperationErrorToast(
        "load",
        t,
        "config read failed token=advanced-secret",
      ),
    ).toEqual({
      title: "加载记忆搜索调优失败",
      description: "详细信息：config read failed token=[redacted]",
    })
    expect(
      memoryAdvancedConfigOperationErrorToast("saveHybrid", t, "permission denied"),
    ).toEqual({
      title: "保存混合搜索调优失败",
      description: "详细信息：permission denied",
    })
    expect(memoryAdvancedConfigOperationErrorToast("saveSelection", t, "bad threshold")).toEqual({
      title: "保存 LLM 记忆选择调优失败",
      description: "详细信息：bad threshold",
    })
    expect(
      memoryAdvancedConfigOperationErrorToast("saveTemporal", t, "half life token=temporal-secret"),
    ).toEqual({
      title: "保存时间衰减调优失败",
      description: "详细信息：half life token=[redacted]",
    })
  })

  it("uses English fallback titles and omits empty detail", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(memoryAdvancedConfigOperationErrorToast("saveDedup", t, "database is locked")).toEqual({
      title: "Failed to save dedup thresholds",
      description: "Details: database is locked",
    })
    expect(memoryAdvancedConfigOperationErrorToast("saveMmr", t, "   ")).toEqual({
      title: "Failed to save MMR tuning",
    })
    expect(memoryAdvancedConfigOperationErrorToast("saveMultimodal", t, null)).toEqual({
      title: "Failed to save multimodal memory tuning",
    })
  })
})
