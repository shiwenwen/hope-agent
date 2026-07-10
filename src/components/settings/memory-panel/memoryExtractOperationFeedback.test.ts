import { describe, expect, it } from "vitest"

import {
  memoryExtractDiagnosticText,
  memoryExtractOperationError,
  memoryExtractOperationErrorDetail,
} from "./memoryExtractOperationFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("memory extract operation feedback", () => {
  it("extracts and redacts user-facing error detail", () => {
    expect(memoryExtractOperationErrorDetail(new Error("config locked"))).toBe("config locked")
    expect(memoryExtractOperationErrorDetail("  provider missing  ")).toBe("provider missing")
    expect(memoryExtractOperationErrorDetail("   ")).toBeNull()
    expect(memoryExtractOperationErrorDetail(null)).toBeNull()
    expect(memoryExtractOperationErrorDetail(undefined)).toBeNull()
    expect(
      memoryExtractDiagnosticText(
        "extract config failed https://api.example.test?token=query-secret Authorization: Bearer bearer-secret api_key=sk-live-secret",
      ),
    ).toBe(
      "extract config failed https://api.example.test?token=[redacted] Authorization: Bearer [redacted] api_key=[redacted]",
    )
    expect(
      memoryExtractOperationErrorDetail(
        "extract save failed password: db-secret passphrase=backup-secret",
      ),
    ).toBe("extract save failed password: [redacted] passphrase=[redacted]")
  })

  it("formats localized operation errors with redacted detail", () => {
    const translations: Record<string, string> = {
      "settings.memoryExtract.errors.saveAgent": "保存 Agent 记忆学习覆盖失败",
      "settings.memoryExtract.errors.detail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(memoryExtractOperationError("saveAgent", t, "agent config locked")).toEqual({
      title: "保存 Agent 记忆学习覆盖失败",
      description: "详细信息：agent config locked",
    })
    expect(
      memoryExtractOperationError("saveAgent", t, "save failed token=extract-secret"),
    ).toEqual({
      title: "保存 Agent 记忆学习覆盖失败",
      description: "详细信息：save failed token=[redacted]",
    })
  })

  it("uses English fallback titles and omits empty detail", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(memoryExtractOperationError("load", t, "backend unavailable")).toEqual({
      title: "Failed to load memory learning settings",
      description: "Details: backend unavailable",
    })
    expect(memoryExtractOperationError("resetAgent", t, "   ")).toEqual({
      title: "Failed to reset agent memory learning override",
    })
  })
})
