import { describe, expect, it } from "vitest"

import {
  memoryBudgetDiagnosticText,
  memoryBudgetOperationErrorDetail,
  memoryBudgetOperationErrorToast,
} from "./memoryBudgetOperationFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("memory budget operation feedback", () => {
  it("extracts user-facing error detail", () => {
    expect(memoryBudgetOperationErrorDetail(new Error("invalid total chars"))).toBe(
      "invalid total chars",
    )
    expect(memoryBudgetOperationErrorDetail("  config locked  ")).toBe("config locked")
    expect(memoryBudgetOperationErrorDetail("   ")).toBeNull()
    expect(memoryBudgetOperationErrorDetail(null)).toBeNull()
    expect(memoryBudgetOperationErrorDetail(undefined)).toBeNull()
    expect(
      memoryBudgetDiagnosticText(
        "budget failed https://api.example.test?token=query-secret Authorization: Bearer bearer-secret api_key=sk-live-secret",
      ),
    ).toBe(
      "budget failed https://api.example.test?token=[redacted] Authorization: Bearer [redacted] api_key=[redacted]",
    )
    expect(
      memoryBudgetOperationErrorDetail(
        "budget save failed password=db-secret passphrase=backup-secret",
      ),
    ).toBe("budget save failed password=[redacted] passphrase=[redacted]")
  })

  it("formats localized operation errors with redacted detail", () => {
    const translations: Record<string, string> = {
      "settings.memoryBudget.errors.save": "保存记忆预算失败",
      "settings.memoryBudget.errors.detail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(memoryBudgetOperationErrorToast("save", t, "total too low")).toEqual({
      title: "保存记忆预算失败",
      description: "详细信息：total too low",
    })
    expect(memoryBudgetOperationErrorToast("save", t, "total too low token=budget-secret")).toEqual(
      {
        title: "保存记忆预算失败",
        description: "详细信息：total too low token=[redacted]",
      },
    )
  })

  it("uses English fallback titles and omits empty detail", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(memoryBudgetOperationErrorToast("load", t, "backend unavailable")).toEqual({
      title: "Failed to load memory budget",
      description: "Details: backend unavailable",
    })
    expect(memoryBudgetOperationErrorToast("save", t, "   ")).toEqual({
      title: "Failed to save memory budget",
    })
  })
})
