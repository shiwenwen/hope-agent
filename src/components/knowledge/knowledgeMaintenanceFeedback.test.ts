import { describe, expect, it } from "vitest"

import {
  knowledgeMaintenanceErrorDetail,
  knowledgeMaintenanceErrorToast,
} from "./knowledgeMaintenanceFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("knowledge maintenance feedback", () => {
  it("extracts and redacts maintenance diagnostics", () => {
    expect(knowledgeMaintenanceErrorDetail(new Error("maintenance locked"))).toBe(
      "maintenance locked",
    )
    expect(
      knowledgeMaintenanceErrorDetail(
        "maintenance failed https://api.example.test?token=query-secret Authorization: Bearer bearer-secret api_key=maint-secret",
      ),
    ).toBe(
      "maintenance failed https://api.example.test?token=[redacted] Authorization: Bearer [redacted] api_key=[redacted]",
    )
    expect(knowledgeMaintenanceErrorDetail("  denied  ")).toBe("denied")
    expect(knowledgeMaintenanceErrorDetail("   ")).toBeNull()
    expect(knowledgeMaintenanceErrorDetail(undefined)).toBeNull()
  })

  it("formats localized maintenance operation errors with redacted detail", () => {
    const translations: Record<string, string> = {
      "knowledge.maintenance.runFailed": "维护运行失败",
      "knowledge.maintenance.evidenceRebuildFailed": "无法重建证据索引",
      "knowledge.maintenance.applyFailed": "应用建议失败",
      "knowledge.maintenance.rejectFailed": "忽略建议失败",
      "knowledge.maintenance.rejectAllFailed": "无法忽略全部建议",
      "knowledge.maintenance.errorDetail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(
      knowledgeMaintenanceErrorToast("runNow", t, "runner token=maintenance-secret"),
    ).toEqual({
      title: "维护运行失败",
      description: "详细信息：runner token=[redacted]",
    })
    expect(
      knowledgeMaintenanceErrorToast(
        "rebuildEvidence",
        t,
        "evidence failed Authorization: Bearer evidence-secret",
      ),
    ).toEqual({
      title: "无法重建证据索引",
      description: "详细信息：evidence failed Authorization: Bearer [redacted]",
    })
    expect(
      knowledgeMaintenanceErrorToast("applyProposal", t, "stale write api_key=apply-secret"),
    ).toEqual({
      title: "应用建议失败",
      description: "详细信息：stale write api_key=[redacted]",
    })
    expect(knowledgeMaintenanceErrorToast("rejectProposal", t, "reject denied")).toEqual({
      title: "忽略建议失败",
      description: "详细信息：reject denied",
    })
    expect(knowledgeMaintenanceErrorToast("rejectAll", t, null)).toEqual({
      title: "无法忽略全部建议",
    })
  })

  it("uses English fallbacks when translations are missing", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(knowledgeMaintenanceErrorToast("rejectAll", t, "denied")).toEqual({
      title: "Couldn't dismiss all suggestions",
      description: "Details: denied",
    })
  })
})
