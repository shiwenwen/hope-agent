import { describe, expect, it } from "vitest"

import {
  knowledgeChunkOperationErrorToast,
  knowledgeCompileAgentOperationErrorToast,
  knowledgeMediaRetentionOperationErrorToast,
  knowledgePanelErrorDetail,
  knowledgePanelOperationErrorText,
  knowledgePanelOperationErrorToast,
  knowledgePassiveRecallOperationErrorToast,
  knowledgeSearchRankingOperationErrorToast,
} from "./knowledgePanelFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("knowledge panel feedback", () => {
  it("extracts and redacts knowledge embedding diagnostics", () => {
    expect(knowledgePanelErrorDetail(new Error("knowledge embedding config locked"))).toBe(
      "knowledge embedding config locked",
    )
    expect(
      knowledgePanelErrorDetail(
        "provider failed https://api.example.test/embed?api_key=query-secret Authorization: Bearer bearer-secret token=inline-secret sk-testsecret123",
      ),
    ).toBe(
      "provider failed https://api.example.test/embed?api_key=[redacted] Authorization: Bearer [redacted] token=[redacted] sk-[redacted]",
    )
    expect(knowledgePanelErrorDetail("  denied  ")).toBe("denied")
    expect(knowledgePanelErrorDetail("   ")).toBeNull()
    expect(knowledgePanelErrorDetail(null)).toBeNull()
  })

  it("formats localized operation errors", () => {
    const translations: Record<string, string> = {
      "settings.knowledgeEmbedding.errors.loadEmbedding": "加载知识向量设置失败",
      "settings.knowledgeEmbedding.errors.activateEmbedding": "启用知识向量检索失败",
      "settings.knowledgeEmbedding.errors.disableEmbedding": "关闭知识向量检索失败",
      "settings.knowledgeEmbedding.errors.rebuildEmbedding": "启动知识向量重建失败",
      "settings.knowledgeEmbedding.errors.cancelReembed": "取消知识向量重建失败",
      "settings.knowledgeEmbedding.errors.retryReembed": "重试知识向量重建失败",
      "settings.knowledgeEmbedding.errors.detail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(
      knowledgePanelOperationErrorToast(
        "activateEmbedding",
        t,
        "provider token=knowledge-secret",
      ),
    ).toEqual({
      title: "启用知识向量检索失败",
      description: "详细信息：provider token=[redacted]",
    })
    expect(knowledgePanelOperationErrorToast("loadEmbedding", t, "db busy")).toEqual({
      title: "加载知识向量设置失败",
      description: "详细信息：db busy",
    })
    expect(knowledgePanelOperationErrorToast("disableEmbedding", t, "   ")).toEqual({
      title: "关闭知识向量检索失败",
    })
  })

  it("uses English fallback text for inline errors", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(
      knowledgePanelOperationErrorText(
        "rebuildEmbedding",
        t,
        "queue full api_key=queue-secret",
      ),
    ).toBe("Failed to start knowledge vector rebuild\nDetails: queue full api_key=[redacted]")
    expect(knowledgePanelOperationErrorToast("cancelReembed", t, null)).toEqual({
      title: "Failed to cancel knowledge vector rebuild",
    })
    expect(knowledgePanelOperationErrorToast("retryReembed", t, "denied")).toEqual({
      title: "Failed to retry knowledge vector rebuild",
      description: "Details: denied",
    })
  })

  it("formats source-to-note agent setting errors", () => {
    const translations: Record<string, string> = {
      "settings.knowledgeCompile.errors.loadConfig": "加载资料整理 Agent 设置失败",
      "settings.knowledgeCompile.errors.loadAgents": "加载资料整理 Agent 列表失败",
      "settings.knowledgeCompile.errors.saveAgent": "保存资料整理 Agent 设置失败",
      "settings.knowledgeCompile.errors.detail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(
      knowledgeCompileAgentOperationErrorToast(
        "loadAgents",
        t,
        "agent list token=agent-secret",
      ),
    ).toEqual({
      title: "加载资料整理 Agent 列表失败",
      description: "详细信息：agent list token=[redacted]",
    })
    expect(knowledgeCompileAgentOperationErrorToast("saveAgent", t, null)).toEqual({
      title: "保存资料整理 Agent 设置失败",
    })
  })

  it("uses English fallback text for source-to-note agent errors", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(
      knowledgeCompileAgentOperationErrorToast(
        "loadConfig",
        t,
        "config api_key=compile-secret",
      ),
    ).toEqual({
      title: "Failed to load source-to-note agent setting",
      description: "Details: config api_key=[redacted]",
    })
  })

  it("formats passive related notes setting errors", () => {
    const translations: Record<string, string> = {
      "settings.knowledgePassiveRecall.errors.load": "加载被动相关笔记设置失败",
      "settings.knowledgePassiveRecall.errors.save": "保存被动相关笔记设置失败",
      "settings.knowledgePassiveRecall.errors.toggle": "更新被动相关笔记开关失败",
      "settings.knowledgePassiveRecall.errors.detail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(
      knowledgePassiveRecallOperationErrorToast(
        "toggle",
        t,
        "toggle token=passive-secret",
      ),
    ).toEqual({
      title: "更新被动相关笔记开关失败",
      description: "详细信息：toggle token=[redacted]",
    })
    expect(knowledgePassiveRecallOperationErrorToast("save", t, null)).toEqual({
      title: "保存被动相关笔记设置失败",
    })
  })

  it("uses English fallback text for passive related notes errors", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(
      knowledgePassiveRecallOperationErrorToast(
        "load",
        t,
        "load api_key=passive-secret",
      ),
    ).toEqual({
      title: "Failed to load passive related notes setting",
      description: "Details: load api_key=[redacted]",
    })
  })

  it("formats knowledge search ranking setting errors", () => {
    const translations: Record<string, string> = {
      "settings.knowledgeSearch.errors.load": "加载知识检索排序设置失败",
      "settings.knowledgeSearch.errors.save": "保存知识检索排序设置失败",
      "settings.knowledgeSearch.errors.restore": "恢复知识检索排序默认值失败",
      "settings.knowledgeSearch.errors.detail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(
      knowledgeSearchRankingOperationErrorToast(
        "restore",
        t,
        "restore token=search-secret",
      ),
    ).toEqual({
      title: "恢复知识检索排序默认值失败",
      description: "详细信息：restore token=[redacted]",
    })
    expect(knowledgeSearchRankingOperationErrorToast("save", t, null)).toEqual({
      title: "保存知识检索排序设置失败",
    })
  })

  it("uses English fallback text for knowledge search ranking errors", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(
      knowledgeSearchRankingOperationErrorToast(
        "load",
        t,
        "ranking api_key=search-secret",
      ),
    ).toEqual({
      title: "Failed to load knowledge search ranking settings",
      description: "Details: ranking api_key=[redacted]",
    })
  })

  it("formats knowledge chunking setting errors", () => {
    const translations: Record<string, string> = {
      "settings.knowledgeChunk.errors.load": "加载知识分块设置失败",
      "settings.knowledgeChunk.errors.save": "保存知识分块设置失败",
      "settings.knowledgeChunk.errors.detail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(
      knowledgeChunkOperationErrorToast("load", t, "chunk token=chunk-secret"),
    ).toEqual({
      title: "加载知识分块设置失败",
      description: "详细信息：chunk token=[redacted]",
    })
    expect(knowledgeChunkOperationErrorToast("save", t, null)).toEqual({
      title: "保存知识分块设置失败",
    })
  })

  it("uses English fallback text for knowledge chunking errors", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(
      knowledgeChunkOperationErrorToast("save", t, "chunk api_key=chunk-secret"),
    ).toEqual({
      title: "Failed to save knowledge chunking settings",
      description: "Details: chunk api_key=[redacted]",
    })
  })

  it("formats original media retention setting errors", () => {
    const translations: Record<string, string> = {
      "settings.knowledgeMediaRetention.errors.load": "加载原始媒体留存设置失败",
      "settings.knowledgeMediaRetention.errors.save": "保存原始媒体留存设置失败",
      "settings.knowledgeMediaRetention.errors.toggle": "更新原始媒体留存开关失败",
      "settings.knowledgeMediaRetention.errors.detail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(
      knowledgeMediaRetentionOperationErrorToast(
        "toggle",
        t,
        "media token=media-secret",
      ),
    ).toEqual({
      title: "更新原始媒体留存开关失败",
      description: "详细信息：media token=[redacted]",
    })
    expect(knowledgeMediaRetentionOperationErrorToast("save", t, null)).toEqual({
      title: "保存原始媒体留存设置失败",
    })
  })

  it("uses English fallback text for original media retention errors", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(
      knowledgeMediaRetentionOperationErrorToast(
        "load",
        t,
        "media api_key=media-secret",
      ),
    ).toEqual({
      title: "Failed to load original media retention settings",
      description: "Details: media api_key=[redacted]",
    })
  })
})
