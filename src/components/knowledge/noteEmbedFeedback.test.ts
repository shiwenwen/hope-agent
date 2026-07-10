import { describe, expect, it } from "vitest"

import {
  knowledgeEmbedErrorDescription,
  knowledgeEmbedErrorDetail,
} from "./noteEmbedFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("note embed feedback", () => {
  it("extracts and redacts resolver diagnostics", () => {
    expect(knowledgeEmbedErrorDetail(new Error("kb read failed"))).toBe("kb read failed")
    expect(
      knowledgeEmbedErrorDetail(
        "read failed Authorization: Bearer note-secret token=query-secret api_key=db-secret",
      ),
    ).toBe(
      "read failed Authorization: Bearer [redacted] token=[redacted] api_key=[redacted]",
    )
    expect(knowledgeEmbedErrorDetail("  denied  ")).toBe("denied")
    expect(knowledgeEmbedErrorDetail("   ")).toBeNull()
    expect(knowledgeEmbedErrorDetail(undefined)).toBeNull()
  })

  it("formats localized error detail", () => {
    const translations: Record<string, string> = {
      "knowledge.embed.errorDetail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(knowledgeEmbedErrorDescription(t, "load token=[redacted]")).toBe(
      "详细信息：load token=[redacted]",
    )
    expect(knowledgeEmbedErrorDescription(t, null)).toBeNull()
  })

  it("uses English fallback when translations are missing", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(knowledgeEmbedErrorDescription(t, "permission denied")).toBe(
      "Details: permission denied",
    )
  })
})
