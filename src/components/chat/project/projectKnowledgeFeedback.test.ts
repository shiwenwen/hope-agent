import { describe, expect, it } from "vitest"

import {
  projectKnowledgeErrorDetail,
  projectKnowledgeLoadErrorToast,
  projectKnowledgeUpdateErrorToast,
} from "./projectKnowledgeFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("project knowledge feedback", () => {
  it("redacts project knowledge error detail", () => {
    expect(projectKnowledgeErrorDetail(new Error("sqlite busy"))).toBe("sqlite busy")
    expect(
      projectKnowledgeErrorDetail(
        "load failed Authorization: Bearer bearer-secret token=query-secret api_key=sk-live-secret",
      ),
    ).toBe(
      "load failed Authorization: Bearer [redacted] token=[redacted] api_key=[redacted]",
    )
    expect(projectKnowledgeErrorDetail("   ")).toBeNull()
  })

  it("formats localized load and update failures", () => {
    const translations: Record<string, string> = {
      "project.knowledge.loadFailed": "加载项目知识空间失败",
      "project.knowledge.updateFailed": "更新项目知识空间失败",
      "project.knowledge.errorDetail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(projectKnowledgeLoadErrorToast(t, "database token=project-secret")).toEqual({
      title: "加载项目知识空间失败",
      description: "详细信息：database token=[redacted]",
    })
    expect(projectKnowledgeUpdateErrorToast(t, "permission denied")).toEqual({
      title: "更新项目知识空间失败",
      description: "详细信息：permission denied",
    })
  })

  it("uses English fallback titles without empty details", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(projectKnowledgeLoadErrorToast(t, null)).toEqual({
      title: "Failed to load project knowledge spaces",
    })
  })
})
