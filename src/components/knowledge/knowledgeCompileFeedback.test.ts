import { describe, expect, it } from "vitest"

import {
  knowledgeCompileErrorDetail,
  knowledgeCompileOperationErrorToast,
} from "./knowledgeCompileFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("knowledge compile feedback", () => {
  it("extracts and redacts compile diagnostics", () => {
    expect(knowledgeCompileErrorDetail(new Error("compile queue locked"))).toBe(
      "compile queue locked",
    )
    expect(
      knowledgeCompileErrorDetail(
        "compile failed https://api.example.test/source?token=query-secret Authorization: Bearer bearer-secret api_key=source-secret",
      ),
    ).toBe(
      "compile failed https://api.example.test/source?token=[redacted] Authorization: Bearer [redacted] api_key=[redacted]",
    )
    expect(knowledgeCompileErrorDetail("  timeout  ")).toBe("timeout")
    expect(knowledgeCompileErrorDetail("   ")).toBeNull()
    expect(knowledgeCompileErrorDetail(undefined)).toBeNull()
  })

  it("formats localized compile operation errors with redacted detail", () => {
    const translations: Record<string, string> = {
      "knowledge.compile.loadFailed": "无法加载资料整理记录",
      "knowledge.compile.proposalsLoadFailed": "无法加载建议",
      "knowledge.compile.startFailed": "无法把资料整理成笔记",
      "knowledge.compile.failed": "资料整理失败",
      "knowledge.compile.cancelFailed": "无法取消运行",
      "knowledge.compile.applyFailed": "无法应用建议",
      "knowledge.compile.rejectFailed": "无法忽略建议",
      "knowledge.compile.errorDetail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(knowledgeCompileOperationErrorToast("loadRuns", t, "db locked")).toEqual({
      title: "无法加载资料整理记录",
      description: "详细信息：db locked",
    })
    expect(
      knowledgeCompileOperationErrorToast("loadProposals", t, "proposals token=proposal-secret"),
    ).toEqual({
      title: "无法加载建议",
      description: "详细信息：proposals token=[redacted]",
    })
    expect(
      knowledgeCompileOperationErrorToast(
        "startCompile",
        t,
        "start failed Authorization: Bearer start-secret",
      ),
    ).toEqual({
      title: "无法把资料整理成笔记",
      description: "详细信息：start failed Authorization: Bearer [redacted]",
    })
    expect(
      knowledgeCompileOperationErrorToast("runFailed", t, "provider failed api_key=run-secret"),
    ).toEqual({
      title: "资料整理失败",
      description: "详细信息：provider failed api_key=[redacted]",
    })
    expect(knowledgeCompileOperationErrorToast("cancelRun", t, null)).toEqual({
      title: "无法取消运行",
    })
    expect(
      knowledgeCompileOperationErrorToast("applyProposal", t, "stale write token=apply-secret"),
    ).toEqual({
      title: "无法应用建议",
      description: "详细信息：stale write token=[redacted]",
    })
    expect(knowledgeCompileOperationErrorToast("rejectProposal", t, "reject denied")).toEqual({
      title: "无法忽略建议",
      description: "详细信息：reject denied",
    })
  })

  it("uses English fallbacks when translations are missing", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(knowledgeCompileOperationErrorToast("applyProposal", t, "denied")).toEqual({
      title: "Couldn't apply proposal",
      description: "Details: denied",
    })
  })
})
