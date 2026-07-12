import { describe, expect, it } from "vitest"

import { knowledgeGraphErrorDetail, knowledgeGraphErrorToast } from "./knowledgeGraphFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("knowledge graph feedback", () => {
  it("extracts and redacts graph diagnostics", () => {
    expect(knowledgeGraphErrorDetail(new Error("graph query failed"))).toBe("graph query failed")
    expect(
      knowledgeGraphErrorDetail(
        "graph failed Authorization: Bearer graph-secret token=query-secret api_key=layout-secret",
      ),
    ).toBe(
      "graph failed Authorization: Bearer [redacted] token=[redacted] api_key=[redacted]",
    )
    expect(knowledgeGraphErrorDetail("  failed  ")).toBe("failed")
    expect(knowledgeGraphErrorDetail("   ")).toBeNull()
    expect(knowledgeGraphErrorDetail(undefined)).toBeNull()
  })

  it("formats localized graph errors with redacted detail", () => {
    const translations: Record<string, string> = {
      "knowledge.graph.loadFailed": "无法加载知识图谱",
      "knowledge.graph.saveLayoutFailed": "无法保存图谱布局",
      "knowledge.graph.resetLayoutFailed": "无法重置图谱布局",
      "knowledge.graph.errorDetail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(knowledgeGraphErrorToast("loadGraph", t, "load token=load-secret")).toEqual({
      title: "无法加载知识图谱",
      description: "详细信息：load token=[redacted]",
    })
    expect(
      knowledgeGraphErrorToast("saveLayout", t, "save Authorization: Bearer save-secret"),
    ).toEqual({
      title: "无法保存图谱布局",
      description: "详细信息：save Authorization: Bearer [redacted]",
    })
    expect(knowledgeGraphErrorToast("resetLayout", t, null)).toEqual({
      title: "无法重置图谱布局",
    })
  })

  it("uses English fallbacks when translations are missing", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(knowledgeGraphErrorToast("loadGraph", t, "denied")).toEqual({
      title: "Couldn't load knowledge graph",
      description: "Details: denied",
    })
  })
})
