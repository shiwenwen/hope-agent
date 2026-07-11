import { describe, expect, it } from "vitest"

import {
  knowledgeQueryFilingErrorDetail,
  knowledgeQueryFilingErrorToast,
} from "./knowledgeQueryFilingFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("knowledge query filing feedback", () => {
  it("extracts and redacts query filing diagnostics", () => {
    expect(knowledgeQueryFilingErrorDetail(new Error("proposal stale"))).toBe("proposal stale")
    expect(
      knowledgeQueryFilingErrorDetail(
        "query file failed https://api.example.test?token=query-secret Authorization: Bearer bearer-secret api_key=file-secret",
      ),
    ).toBe(
      "query file failed https://api.example.test?token=[redacted] Authorization: Bearer [redacted] api_key=[redacted]",
    )
    expect(knowledgeQueryFilingErrorDetail("  denied  ")).toBe("denied")
    expect(knowledgeQueryFilingErrorDetail("   ")).toBeNull()
    expect(knowledgeQueryFilingErrorDetail(undefined)).toBeNull()
  })

  it("formats localized query filing errors with redacted detail", () => {
    const translations: Record<string, string> = {
      "knowledge.queryFile.generateFailed": "无法创建归档提案",
      "knowledge.queryFile.applyFailed": "无法应用归档",
      "knowledge.queryFile.rejectFailed": "无法丢弃归档",
      "knowledge.queryFile.errorDetail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(
      knowledgeQueryFilingErrorToast("generateProposal", t, "provider token=generate-secret"),
    ).toEqual({
      title: "无法创建归档提案",
      description: "详细信息：provider token=[redacted]",
    })
    expect(
      knowledgeQueryFilingErrorToast(
        "applyFiling",
        t,
        "stale write Authorization: Bearer apply-secret",
      ),
    ).toEqual({
      title: "无法应用归档",
      description: "详细信息：stale write Authorization: Bearer [redacted]",
    })
    expect(knowledgeQueryFilingErrorToast("rejectFiling", t, null)).toEqual({
      title: "无法丢弃归档",
    })
  })

  it("uses English fallbacks when translations are missing", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(knowledgeQueryFilingErrorToast("generateProposal", t, "denied")).toEqual({
      title: "Couldn't create filing proposal",
      description: "Details: denied",
    })
  })
})
