import { describe, expect, it } from "vitest"

import {
  knowledgeFocusErrorDescription,
  knowledgeFocusErrorDetail,
} from "./knowledgeFocusFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("knowledge focus feedback", () => {
  it("extracts and redacts source focus error detail", () => {
    expect(knowledgeFocusErrorDetail(new Error("note read failed"))).toBe("note read failed")
    expect(
      knowledgeFocusErrorDetail(
        "note read failed Authorization: Bearer bearer-secret api_key=kb-secret",
      ),
    ).toBe("note read failed Authorization: Bearer [redacted] api_key=[redacted]")
    expect(knowledgeFocusErrorDetail("   ")).toBeNull()
    expect(knowledgeFocusErrorDetail(null)).toBeNull()
  })

  it("formats localized detail", () => {
    const translations: Record<string, string> = {
      "knowledge.focusUnavailableDetail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(knowledgeFocusErrorDescription(t, "database token=note-secret")).toBe(
      "详细信息：database token=[redacted]",
    )
  })
})
