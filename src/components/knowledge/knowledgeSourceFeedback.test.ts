import { describe, expect, it } from "vitest"

import { knowledgeSourceErrorDetail, knowledgeSourceErrorMessage } from "./knowledgeSourceFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("knowledge source feedback", () => {
  it("extracts and redacts diagnostic details", () => {
    expect(knowledgeSourceErrorDetail(new Error("source read locked"))).toBe("source read locked")
    expect(
      knowledgeSourceErrorDetail(
        "claims failed https://api.example.test/source?token=query-secret Authorization: Bearer bearer-secret api_key=source-secret",
      ),
    ).toBe(
      "claims failed https://api.example.test/source?token=[redacted] Authorization: Bearer [redacted] api_key=[redacted]",
    )
    expect(knowledgeSourceErrorDetail("  timeout  ")).toBe("timeout")
    expect(knowledgeSourceErrorDetail("   ")).toBeNull()
    expect(knowledgeSourceErrorDetail(undefined)).toBeNull()
  })

  it("formats localized source claim reference errors", () => {
    const translations: Record<string, string> = {
      "knowledge.sources.sourceListFailed": "无法加载资料列表",
      "knowledge.sources.importRunsListFailed": "无法加载导入历史",
      "knowledge.sources.similarGroupsLoadFailed": "无法加载相似资料分组",
      "knowledge.sources.importFailed": "无法导入资料",
      "knowledge.sources.importHistoryFailed": "无法打开导入历史",
      "knowledge.sources.retryFailed": "无法重试失败导入",
      "knowledge.sources.similarDismissFailed": "无法隐藏相似建议",
      "knowledge.sources.similarResolveFailed": "无法处理重复资料",
      "knowledge.sources.deleteFailed": "无法删除资料",
      "knowledge.sources.reextractFailed": "无法重新提取资料",
      "knowledge.sources.refreshFailed": "无法刷新资料",
      "knowledge.sources.versionsFailed": "无法加载资料版本",
      "knowledge.sources.diffFailed": "无法加载资料差异",
      "knowledge.sources.openOriginalFailed": "无法打开原件",
      "knowledge.sources.downloadOriginalFailed": "无法下载原件",
      "knowledge.sources.sourceClaimsFailed": "无法加载资料结论引用",
      "knowledge.sources.sourceErrorDetail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(
      knowledgeSourceErrorMessage(
        "loadSourceClaims",
        t,
        "index unavailable token=claim-secret",
      ),
    ).toEqual({
      title: "无法加载资料结论引用",
      description: "详细信息：index unavailable token=[redacted]",
    })
    expect(knowledgeSourceErrorMessage("loadSources", t, "db locked api_key=list-secret")).toEqual({
      title: "无法加载资料列表",
      description: "详细信息：db locked api_key=[redacted]",
    })
    expect(knowledgeSourceErrorMessage("loadImportRuns", t, null)).toEqual({
      title: "无法加载导入历史",
    })
    expect(
      knowledgeSourceErrorMessage("loadSimilarGroups", t, "permission denied"),
    ).toEqual({
      title: "无法加载相似资料分组",
      description: "详细信息：permission denied",
    })
    expect(
      knowledgeSourceErrorMessage("importSource", t, "import denied token=import-secret"),
    ).toEqual({
      title: "无法导入资料",
      description: "详细信息：import denied token=[redacted]",
    })
    expect(
      knowledgeSourceErrorMessage(
        "openImportHistory",
        t,
        "history failed Authorization: Bearer history-secret",
      ),
    ).toEqual({
      title: "无法打开导入历史",
      description: "详细信息：history failed Authorization: Bearer [redacted]",
    })
    expect(
      knowledgeSourceErrorMessage("retryFailedImport", t, "retry failed api_key=retry-secret"),
    ).toEqual({
      title: "无法重试失败导入",
      description: "详细信息：retry failed api_key=[redacted]",
    })
    expect(
      knowledgeSourceErrorMessage("dismissSimilarGroup", t, "dismiss denied"),
    ).toEqual({
      title: "无法隐藏相似建议",
      description: "详细信息：dismiss denied",
    })
    expect(
      knowledgeSourceErrorMessage("resolveSimilarGroup", t, "resolve denied"),
    ).toEqual({
      title: "无法处理重复资料",
      description: "详细信息：resolve denied",
    })
    expect(
      knowledgeSourceErrorMessage("deleteSource", t, "delete denied api_key=delete-secret"),
    ).toEqual({
      title: "无法删除资料",
      description: "详细信息：delete denied api_key=[redacted]",
    })
    expect(
      knowledgeSourceErrorMessage(
        "reextractSource",
        t,
        "reextract failed Authorization: Bearer reextract-secret",
      ),
    ).toEqual({
      title: "无法重新提取资料",
      description: "详细信息：reextract failed Authorization: Bearer [redacted]",
    })
    expect(
      knowledgeSourceErrorMessage("refreshSource", t, "refresh failed token=refresh-secret"),
    ).toEqual({
      title: "无法刷新资料",
      description: "详细信息：refresh failed token=[redacted]",
    })
    expect(
      knowledgeSourceErrorMessage(
        "loadSourceVersions",
        t,
        "versions denied Authorization: Bearer versions-secret",
      ),
    ).toEqual({
      title: "无法加载资料版本",
      description: "详细信息：versions denied Authorization: Bearer [redacted]",
    })
    expect(
      knowledgeSourceErrorMessage("loadSourceDiff", t, "diff failed token=diff-secret"),
    ).toEqual({
      title: "无法加载资料差异",
      description: "详细信息：diff failed token=[redacted]",
    })
    expect(
      knowledgeSourceErrorMessage(
        "openOriginalAsset",
        t,
        "open failed Authorization: Bearer original-secret",
      ),
    ).toEqual({
      title: "无法打开原件",
      description: "详细信息：open failed Authorization: Bearer [redacted]",
    })
    expect(
      knowledgeSourceErrorMessage(
        "downloadOriginalAsset",
        t,
        "download failed token=asset-secret",
      ),
    ).toEqual({
      title: "无法下载原件",
      description: "详细信息：download failed token=[redacted]",
    })
  })

  it("formats source read failures and omits empty detail", () => {
    const translations: Record<string, string> = {
      "knowledge.sources.readFailed": "无法打开资料",
      "knowledge.sources.sourceErrorDetail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(
      knowledgeSourceErrorMessage(
        "readSource",
        t,
        "permission denied Authorization: Bearer source-secret",
      ),
    ).toEqual({
      title: "无法打开资料",
      description: "详细信息：permission denied Authorization: Bearer [redacted]",
    })
    expect(knowledgeSourceErrorMessage("readSource", t, "   ")).toEqual({
      title: "无法打开资料",
    })
  })
})
