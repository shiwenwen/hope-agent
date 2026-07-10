import { describe, expect, it } from "vitest"

import {
  knowledgeEmbeddingLoadErrorDetail,
  knowledgeEmbeddingUnavailableTip,
} from "./knowledgeEmbeddingBadgeFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("knowledge embedding badge feedback", () => {
  it("extracts and redacts load diagnostics", () => {
    expect(knowledgeEmbeddingLoadErrorDetail(new Error("config store locked"))).toBe(
      "config store locked",
    )
    expect(
      knowledgeEmbeddingLoadErrorDetail(
        "failed Authorization: Bearer vector-secret token=query-secret api_key=model-secret",
      ),
    ).toBe(
      "failed Authorization: Bearer [redacted] token=[redacted] api_key=[redacted]",
    )
    expect(knowledgeEmbeddingLoadErrorDetail("  failed  ")).toBe("failed")
    expect(knowledgeEmbeddingLoadErrorDetail("   ")).toBeNull()
    expect(knowledgeEmbeddingLoadErrorDetail(undefined)).toBeNull()
  })

  it("formats localized unavailable tooltip with redacted detail", () => {
    const translations: Record<string, string> = {
      "knowledge.embeddingStatusUnavailableTip": "向量检索状态不可用 · 点击进入设置",
      "knowledge.embeddingStatusErrorDetail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    const detail = knowledgeEmbeddingLoadErrorDetail("load token=load-secret")
    expect(knowledgeEmbeddingUnavailableTip(t, detail)).toBe(
      "向量检索状态不可用 · 点击进入设置 · 详细信息：load token=[redacted]",
    )
    expect(knowledgeEmbeddingUnavailableTip(t, null)).toBe("向量检索状态不可用 · 点击进入设置")
  })

  it("uses English fallbacks when translations are missing", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(knowledgeEmbeddingUnavailableTip(t, "denied")).toBe(
      "Vector search status is unavailable — open settings · Details: denied",
    )
  })
})
