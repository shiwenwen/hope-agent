import { describe, expect, it } from "vitest"

import {
  knowledgeQuickRewriteErrorDetail,
  knowledgeQuickRewriteErrorToast,
  knowledgeQuickRewriteOperationError,
} from "./knowledgeQuickRewriteFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("knowledge quick rewrite feedback", () => {
  it("extracts and redacts rewrite diagnostics", () => {
    expect(knowledgeQuickRewriteErrorDetail(new Error("provider unavailable"))).toBe(
      "provider unavailable",
    )
    expect(
      knowledgeQuickRewriteErrorDetail(
        "rewrite failed https://api.example.test?token=query-secret Authorization: Bearer bearer-secret api_key=rewrite-secret",
      ),
    ).toBe(
      "rewrite failed https://api.example.test?token=[redacted] Authorization: Bearer [redacted] api_key=[redacted]",
    )
    expect(knowledgeQuickRewriteErrorDetail("  timeout  ")).toBe("timeout")
    expect(knowledgeQuickRewriteErrorDetail("   ")).toBeNull()
    expect(knowledgeQuickRewriteErrorDetail(undefined)).toBeNull()
  })

  it("formats localized quick rewrite errors with redacted detail", () => {
    const translations: Record<string, string> = {
      "knowledge.quickRewrite.failed": "改写失败",
      "knowledge.quickRewrite.modelLoadFailed": "无法加载改写模型",
      "knowledge.quickRewrite.errorDetail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(
      knowledgeQuickRewriteErrorToast(
        t,
        "rewrite denied Authorization: Bearer rewrite-secret",
      ),
    ).toEqual({
      title: "改写失败",
      description: "详细信息：rewrite denied Authorization: Bearer [redacted]",
    })
    expect(knowledgeQuickRewriteErrorToast(t, null)).toEqual({
      title: "改写失败",
    })
    expect(
      knowledgeQuickRewriteOperationError(
        "loadModels",
        t,
        "models failed token=model-secret",
      ),
    ).toEqual({
      title: "无法加载改写模型",
      description: "详细信息：models failed token=[redacted]",
    })
  })

  it("uses English fallbacks when translations are missing", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(knowledgeQuickRewriteErrorToast(t, "provider token=rewrite-secret")).toEqual({
      title: "Rewrite failed",
      description: "Details: provider token=[redacted]",
    })
    expect(knowledgeQuickRewriteOperationError("loadModels", t, "settings denied")).toEqual({
      title: "Couldn't load rewrite models",
      description: "Details: settings denied",
    })
  })
})
