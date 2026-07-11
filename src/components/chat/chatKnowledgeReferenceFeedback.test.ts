import { describe, expect, it } from "vitest"

import {
  chatKnowledgeReferenceAttachErrorToast,
  chatKnowledgeReferenceErrorDetail,
} from "./chatKnowledgeReferenceFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("chat knowledge reference feedback", () => {
  it("redacts knowledge auto-attach errors", () => {
    expect(chatKnowledgeReferenceErrorDetail(new Error("sqlite busy"))).toBe("sqlite busy")
    expect(
      chatKnowledgeReferenceErrorDetail(
        "attach failed Authorization: Bearer bearer-secret token=query-secret api_key=sk-live-secret",
      ),
    ).toBe(
      "attach failed Authorization: Bearer [redacted] token=[redacted] api_key=[redacted]",
    )
    expect(chatKnowledgeReferenceErrorDetail("   ")).toBeNull()
  })

  it("formats localized attach failure toasts", () => {
    const translations: Record<string, string> = {
      "chat.knowledgeReferenceAttachFailed": "知识空间自动挂载失败",
      "chat.knowledgeReferenceAttachFailedDetail":
        "笔记引用已插入，但助手可能无法读取它。详细信息：{{error}}",
      "chat.knowledgeReferenceAttachFailedHint": "笔记引用已插入，但助手可能无法读取它。",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(chatKnowledgeReferenceAttachErrorToast(t, "permission token=secret")).toEqual({
      title: "知识空间自动挂载失败",
      description: "笔记引用已插入，但助手可能无法读取它。详细信息：permission token=[redacted]",
    })
    expect(chatKnowledgeReferenceAttachErrorToast(t, null)).toEqual({
      title: "知识空间自动挂载失败",
      description: "笔记引用已插入，但助手可能无法读取它。",
    })
  })
})
