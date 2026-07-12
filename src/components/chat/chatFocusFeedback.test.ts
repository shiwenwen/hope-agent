import { describe, expect, it } from "vitest"

import {
  chatFocusErrorDetail,
  chatFocusLoadErrorToast,
  chatFocusMissingMessageToast,
  chatFocusMissingSessionToast,
} from "./chatFocusFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("chat focus feedback", () => {
  it("extracts and redacts source focus error detail", () => {
    expect(chatFocusErrorDetail(new Error("session query failed"))).toBe("session query failed")
    expect(
      chatFocusErrorDetail(
        "source fetch failed Authorization: Bearer bearer-secret api_key=chat-secret",
      ),
    ).toBe(
      "source fetch failed Authorization: Bearer [redacted] api_key=[redacted]",
    )
    expect(chatFocusErrorDetail("   ")).toBeNull()
    expect(chatFocusErrorDetail(null)).toBeNull()
  })

  it("formats localized source load failures", () => {
    const translations: Record<string, string> = {
      "chat.openSourceConversationFailed": "打开来源会话失败",
      "chat.openSourceConversationFailedDetail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(chatFocusLoadErrorToast(t, "database token=chat-secret")).toEqual({
      title: "打开来源会话失败",
      description: "详细信息：database token=[redacted]",
    })
  })

  it("formats missing-source fallbacks", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(chatFocusMissingSessionToast(t)).toEqual({
      title: "Source conversation is no longer available",
      description:
        "This memory source points to a conversation that was deleted or cannot be found.",
    })
    expect(chatFocusMissingMessageToast(t)).toEqual({
      title: "Source message is no longer available",
      description:
        "The conversation still exists, but the exact message for this memory source could not be found.",
    })
  })
})
