import { describe, expect, it } from "vitest"

import {
  coreMemoryDiagnosticText,
  coreMemoryOperationErrorDetail,
  coreMemoryOperationErrorToast,
  coreMemoryOperationForScope,
} from "./coreMemoryOperationFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("core memory operation feedback", () => {
  it("extracts user-facing error detail", () => {
    expect(coreMemoryOperationErrorDetail(new Error("permission denied"))).toBe(
      "permission denied",
    )
    expect(coreMemoryOperationErrorDetail("  disk full  ")).toBe("disk full")
    expect(coreMemoryOperationErrorDetail("   ")).toBeNull()
    expect(coreMemoryOperationErrorDetail(null)).toBeNull()
    expect(coreMemoryOperationErrorDetail(undefined)).toBeNull()
    expect(
      coreMemoryDiagnosticText(
        "core memory failed https://api.example.test?token=query-secret Authorization: Bearer bearer-secret api_key=sk-live-secret",
      ),
    ).toBe(
      "core memory failed https://api.example.test?token=[redacted] Authorization: Bearer [redacted] api_key=[redacted]",
    )
    expect(
      coreMemoryOperationErrorDetail(
        "core memory write failed password=db-secret passphrase=backup-secret",
      ),
    ).toBe("core memory write failed password=[redacted] passphrase=[redacted]")
  })

  it("maps load/save actions to scoped operations", () => {
    expect(coreMemoryOperationForScope("load", "global")).toBe("loadGlobal")
    expect(coreMemoryOperationForScope("load", "agent")).toBe("loadAgent")
    expect(coreMemoryOperationForScope("save", "global")).toBe("saveGlobal")
    expect(coreMemoryOperationForScope("save", "agent")).toBe("saveAgent")
  })

  it("formats localized operation errors with redacted detail", () => {
    const translations: Record<string, string> = {
      "settings.coreMemoryErrors.saveAgent": "保存 Agent 核心记忆失败",
      "settings.coreMemoryErrors.detail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(coreMemoryOperationErrorToast("saveAgent", t, "write failed")).toEqual({
      title: "保存 Agent 核心记忆失败",
      description: "详细信息：write failed",
    })
    expect(coreMemoryOperationErrorToast("saveAgent", t, "write failed token=core-secret")).toEqual(
      {
        title: "保存 Agent 核心记忆失败",
        description: "详细信息：write failed token=[redacted]",
      },
    )
  })

  it("uses English fallback titles and omits empty detail", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(coreMemoryOperationErrorToast("loadGlobal", t, "not found")).toEqual({
      title: "Failed to load global core memory",
      description: "Details: not found",
    })
    expect(coreMemoryOperationErrorToast("saveGlobal", t, "   ")).toEqual({
      title: "Failed to save global core memory",
    })
  })
})
