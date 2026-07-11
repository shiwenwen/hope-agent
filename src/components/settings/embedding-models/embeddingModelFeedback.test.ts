import { describe, expect, it } from "vitest"

import {
  embeddingModelDiagnosticText,
  embeddingModelErrorDetail,
  embeddingModelOperationErrorToast,
} from "./embeddingModelFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("embedding model feedback", () => {
  it("redacts provider credentials from diagnostic text", () => {
    const diagnostic = embeddingModelDiagnosticText(
      "request failed https://api.example.test/embeddings?key=query-secret Authorization: Bearer bearer-secret api_key=sk-live-secret token=inline-secret AIzaSyA123456789012345678901234567890",
    )

    expect(diagnostic).toContain("key=[redacted]")
    expect(diagnostic).toContain("Authorization: Bearer [redacted]")
    expect(diagnostic).toContain("api_key=[redacted]")
    expect(diagnostic).toContain("token=[redacted]")
    expect(diagnostic).toContain("AIza[redacted]")
    expect(diagnostic).not.toContain("query-secret")
    expect(diagnostic).not.toContain("bearer-secret")
    expect(diagnostic).not.toContain("sk-live-secret")
    expect(diagnostic).not.toContain("inline-secret")
    expect(diagnostic).not.toContain("AIzaSyA123456789012345678901234567890")
  })

  it("extracts non-empty error details", () => {
    expect(embeddingModelErrorDetail(new Error("provider unavailable"))).toBe(
      "provider unavailable",
    )
    expect(embeddingModelErrorDetail("connection failed apiKey=secret-key")).toBe(
      "connection failed apiKey=[redacted]",
    )
    expect(embeddingModelErrorDetail("  timeout  ")).toBe("timeout")
    expect(embeddingModelErrorDetail("   ")).toBeNull()
    expect(embeddingModelErrorDetail(null)).toBeNull()
  })

  it("formats localized operation errors while preserving redacted detail", () => {
    const translations: Record<string, string> = {
      "settings.embeddingModels.errors.save": "无法保存 Embedding 模型配置",
      "settings.embeddingModels.errors.detail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(embeddingModelOperationErrorToast("save", t, "invalid API key api_key=secret")).toEqual({
      title: "无法保存 Embedding 模型配置",
      description: "详细信息：invalid API key api_key=[redacted]",
    })
  })

  it("uses English fallback titles and omits empty detail", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(embeddingModelOperationErrorToast("setDefault", t, "database locked")).toEqual({
      title: "Failed to switch default memory model",
      description: "Details: database locked",
    })
    expect(embeddingModelOperationErrorToast("test", t, "   ")).toEqual({
      title: "Embedding connection test failed",
    })
  })
})
