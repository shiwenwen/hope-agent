import { describe, expect, it } from "vitest"

import {
  noteSourceReferenceErrorDetail,
  noteSourceReferenceErrorMessage,
} from "./noteSourceReferenceFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("note source reference feedback", () => {
  it("extracts and redacts diagnostic details", () => {
    expect(noteSourceReferenceErrorDetail(new Error("source index locked"))).toBe(
      "source index locked",
    )
    expect(
      noteSourceReferenceErrorDetail(
        "read failed https://api.example.test/source?token=query-secret Authorization: Bearer bearer-secret api_key=source-secret",
      ),
    ).toBe(
      "read failed https://api.example.test/source?token=[redacted] Authorization: Bearer [redacted] api_key=[redacted]",
    )
    expect(noteSourceReferenceErrorDetail("  missing source  ")).toBe("missing source")
    expect(noteSourceReferenceErrorDetail("   ")).toBeNull()
    expect(noteSourceReferenceErrorDetail(null)).toBeNull()
  })

  it("formats localized source reference load errors", () => {
    const translations: Record<string, string> = {
      "knowledge.sources.sourceRefsFailed": "无法加载资料来源引用",
      "knowledge.sources.sourceErrorDetail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(
      noteSourceReferenceErrorMessage("loadRefs", t, "sqlite locked token=refs-secret"),
    ).toEqual({
      title: "无法加载资料来源引用",
      description: "详细信息：sqlite locked token=[redacted]",
    })
  })

  it("formats source read failures and omits empty detail", () => {
    const translations: Record<string, string> = {
      "knowledge.sources.readFailed": "无法打开资料",
      "knowledge.sources.sourceErrorDetail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(
      noteSourceReferenceErrorMessage(
        "readSource",
        t,
        "permission denied Authorization: Bearer source-secret",
      ),
    ).toEqual({
      title: "无法打开资料",
      description: "详细信息：permission denied Authorization: Bearer [redacted]",
    })
    expect(noteSourceReferenceErrorMessage("readSource", t, "   ")).toEqual({
      title: "无法打开资料",
    })
  })
})
