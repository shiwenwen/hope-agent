import { describe, expect, it } from "vitest"

import {
  isKnowledgeRemoteWriteBlocked,
  knowledgeViewErrorDetail,
  knowledgeViewOperationErrorToast,
} from "./knowledgeViewFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("knowledge view feedback", () => {
  it("extracts and redacts owner operation diagnostics", () => {
    expect(knowledgeViewErrorDetail(new Error("sqlite busy"))).toBe("sqlite busy")
    expect(
      knowledgeViewErrorDetail(
        "failed Authorization: Bearer owner-secret token=query-secret api_key=space-secret",
      ),
    ).toBe(
      "failed Authorization: Bearer [redacted] token=[redacted] api_key=[redacted]",
    )
    expect(knowledgeViewErrorDetail("  failed  ")).toBe("failed")
    expect(knowledgeViewErrorDetail("   ")).toBeNull()
    expect(knowledgeViewErrorDetail(undefined)).toBeNull()
  })

  it("formats localized operation errors with detail", () => {
    const translations: Record<string, string> = {
      "knowledge.errors.loadSpaces": "无法加载知识空间",
      "knowledge.errors.reindexNote": "无法重建笔记索引",
      "knowledge.renameMoveFailed": "无法重命名/移动「{{name}}」",
      "knowledge.operationErrorDetail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(knowledgeViewOperationErrorToast("loadSpaces", t, "list token=list-secret")).toEqual({
      title: "无法加载知识空间",
      description: "详细信息：list token=[redacted]",
    })
    expect(
      knowledgeViewOperationErrorToast("renameMove", t, "rename api_key=rename-secret", {
        name: "Notes/a.md",
      }),
    ).toEqual({
      title: "无法重命名/移动「Notes/a.md」",
      description: "详细信息：rename api_key=[redacted]",
    })
    expect(knowledgeViewOperationErrorToast("reindexNote", t, null)).toEqual({
      title: "无法重建笔记索引",
    })
  })

  it("uses the remote-write title for write operations", () => {
    const translations: Record<string, string> = {
      "knowledge.remoteWritesDisabled": "远程写入已关闭",
      "knowledge.errors.saveNote": "无法保存笔记",
      "knowledge.operationErrorDetail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)
    const error = "remote file writes are disabled token=blocked-secret"

    expect(isKnowledgeRemoteWriteBlocked(error)).toBe(true)
    expect(knowledgeViewOperationErrorToast("saveNote", t, error)).toEqual({
      title: "远程写入已关闭",
      description: "详细信息：remote file writes are disabled token=[redacted]",
    })
    expect(knowledgeViewOperationErrorToast("reindexNote", t, error).title).toBe(
      "Couldn't rebuild note index",
    )
  })

  it("uses English fallbacks when translations are missing", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(knowledgeViewOperationErrorToast("deleteSpace", t, "denied")).toEqual({
      title: "Couldn't delete knowledge space",
      description: "Details: denied",
    })
  })
})
