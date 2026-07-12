import { describe, expect, it } from "vitest"

import {
  knowledgeJobsActionOperation,
  knowledgeJobsErrorDetail,
  knowledgeJobsErrorToast,
} from "./knowledgeJobsFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("knowledge jobs feedback", () => {
  it("extracts and redacts rebuild task diagnostics", () => {
    expect(knowledgeJobsErrorDetail(new Error("job store locked"))).toBe("job store locked")
    expect(
      knowledgeJobsErrorDetail(
        "job failed https://api.example.test?token=query-secret Authorization: Bearer bearer-secret api_key=job-secret",
      ),
    ).toBe(
      "job failed https://api.example.test?token=[redacted] Authorization: Bearer [redacted] api_key=[redacted]",
    )
    expect(knowledgeJobsErrorDetail("  failed  ")).toBe("failed")
    expect(knowledgeJobsErrorDetail("   ")).toBeNull()
    expect(knowledgeJobsErrorDetail(undefined)).toBeNull()
  })

  it("formats localized rebuild task errors with redacted detail", () => {
    const translations: Record<string, string> = {
      "knowledge.jobs.loadFailed": "无法加载重建任务",
      "knowledge.jobs.cancelFailed": "无法取消重建任务",
      "knowledge.jobs.retryFailed": "无法重试重建任务",
      "knowledge.jobs.clearFailed": "无法清除重建任务",
      "knowledge.jobs.errorDetail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(knowledgeJobsErrorToast("loadJobs", t, "list token=list-secret")).toEqual({
      title: "无法加载重建任务",
      description: "详细信息：list token=[redacted]",
    })
    expect(
      knowledgeJobsErrorToast("cancelJob", t, "cancel Authorization: Bearer cancel-secret"),
    ).toEqual({
      title: "无法取消重建任务",
      description: "详细信息：cancel Authorization: Bearer [redacted]",
    })
    expect(knowledgeJobsErrorToast("retryJob", t, "retry api_key=retry-secret")).toEqual({
      title: "无法重试重建任务",
      description: "详细信息：retry api_key=[redacted]",
    })
    expect(knowledgeJobsErrorToast("clearJob", t, null)).toEqual({
      title: "无法清除重建任务",
    })
  })

  it("maps job actions to operation names", () => {
    expect(knowledgeJobsActionOperation("cancel")).toBe("cancelJob")
    expect(knowledgeJobsActionOperation("retry")).toBe("retryJob")
    expect(knowledgeJobsActionOperation("clear")).toBe("clearJob")
  })

  it("uses English fallbacks when translations are missing", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(knowledgeJobsErrorToast("retryJob", t, "denied")).toEqual({
      title: "Couldn't retry rebuild task",
      description: "Details: denied",
    })
  })
})
