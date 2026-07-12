import { describe, expect, it } from "vitest"

import {
  agentLoadOperationErrorDetail,
  agentLoadOperationErrorToast,
  agentOperationErrorToast,
} from "./agentLoadOperationFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("agent load operation feedback", () => {
  it("extracts and redacts user-facing error detail", () => {
    expect(agentLoadOperationErrorDetail(new Error("agent row missing"))).toBe(
      "agent row missing",
    )
    expect(
      agentLoadOperationErrorDetail(
        "agent fetch failed Authorization: Bearer bearer-secret api_key=agent-secret",
      ),
    ).toBe(
      "agent fetch failed Authorization: Bearer [redacted] api_key=[redacted]",
    )
    expect(agentLoadOperationErrorDetail("   ")).toBeNull()
    expect(agentLoadOperationErrorDetail(null)).toBeNull()
  })

  it("formats localized load failure toasts", () => {
    const translations: Record<string, string> = {
      "settings.agentLoadFailed": "加载 Agent 失败",
      "settings.agentLoadFailedDetail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(agentLoadOperationErrorToast(t, "database token=agent-secret")).toEqual({
      title: "加载 Agent 失败",
      description: "详细信息：database token=[redacted]",
    })
  })

  it("uses English fallback without empty detail", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(agentLoadOperationErrorToast(t, "   ")).toEqual({
      title: "Failed to load agent",
    })
  })

  it("formats save and delete failures with localized titles and redacted detail", () => {
    const translations: Record<string, string> = {
      "common.saveFailed": "保存失败",
      "common.deleteFailed": "删除失败",
      "settings.agentLoadFailedDetail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(agentOperationErrorToast("save", t, "write api_key=save-secret")).toEqual({
      title: "保存失败",
      description: "详细信息：write api_key=[redacted]",
    })
    expect(agentOperationErrorToast("delete", t, "delete token=delete-secret")).toEqual({
      title: "删除失败",
      description: "详细信息：delete token=[redacted]",
    })
  })
})
