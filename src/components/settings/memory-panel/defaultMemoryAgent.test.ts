import { describe, expect, it } from "vitest"

import { DEFAULT_AGENT_ID } from "@/types/tools"
import {
  formatMemoryUseInRepliesError,
  memoryUseInRepliesDiagnosticText,
  memoryUseInRepliesErrorDescription,
  memoryUseInRepliesErrorDetail,
  normalizeDefaultMemoryAgentId,
} from "./defaultMemoryAgent"

function t(key: string, options?: Record<string, unknown>): string {
  const translations: Record<string, string> = {
    "settings.memoryUseInRepliesLoadFailed": "无法检查主动召回",
    "settings.memoryUseInRepliesLoadError": "无法检查主动召回：{{error}}",
    "settings.memoryUseInRepliesUpdateFailed": "无法更新主动召回",
    "settings.memoryUseInRepliesError": "无法更新主动召回：{{error}}",
    "settings.memoryUseInRepliesErrorDetail": "详细信息：{{error}}",
  }
  const text = translations[key] ?? String(options?.defaultValue ?? key)
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("normalizeDefaultMemoryAgentId", () => {
  it("uses a trimmed configured default agent id", () => {
    expect(normalizeDefaultMemoryAgentId(" custom-agent ")).toBe("custom-agent")
  })

  it("falls back to the hardcoded main agent when the config value is empty", () => {
    expect(normalizeDefaultMemoryAgentId(null)).toBe(DEFAULT_AGENT_ID)
    expect(normalizeDefaultMemoryAgentId(" ")).toBe(DEFAULT_AGENT_ID)
    expect(normalizeDefaultMemoryAgentId(123)).toBe(DEFAULT_AGENT_ID)
  })

  it("formats active recall load/update failures with action context", () => {
    expect(memoryUseInRepliesErrorDetail(new Error("agent missing"))).toBe("agent missing")
    expect(memoryUseInRepliesErrorDetail("  timeout  ")).toBe("timeout")
    expect(
      memoryUseInRepliesDiagnosticText(
        "active recall failed https://api.example.test?token=query-secret Authorization: Bearer bearer-secret api_key=sk-live-secret",
      ),
    ).toBe(
      "active recall failed https://api.example.test?token=[redacted] Authorization: Bearer [redacted] api_key=[redacted]",
    )
    expect(
      memoryUseInRepliesErrorDetail(
        "agent save failed password=config-secret passphrase=backup-secret",
      ),
    ).toBe("agent save failed password=[redacted] passphrase=[redacted]")
    expect(memoryUseInRepliesErrorDetail("  ")).toBeNull()

    expect(formatMemoryUseInRepliesError(t, "load", new Error("agent missing"))).toBe(
      "无法检查主动召回：agent missing",
    )
    expect(formatMemoryUseInRepliesError(t, "update", "database locked")).toBe(
      "无法更新主动召回：database locked",
    )
    expect(formatMemoryUseInRepliesError(t, "load", " ")).toBe("无法检查主动召回")
    expect(memoryUseInRepliesErrorDescription(t, "database locked")).toBe(
      "详细信息：database locked",
    )
    expect(memoryUseInRepliesErrorDescription(t, " ")).toBeUndefined()
  })
})
