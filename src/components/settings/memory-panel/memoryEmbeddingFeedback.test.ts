import { describe, expect, it } from "vitest"

import {
  memoryEmbeddingErrorDetail,
  memoryEmbeddingOperationErrorText,
  memoryEmbeddingOperationErrorToast,
} from "./memoryEmbeddingFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("memory embedding feedback", () => {
  it("extracts bounded user-facing error detail", () => {
    expect(memoryEmbeddingErrorDetail(new Error("provider unavailable"))).toBe(
      "provider unavailable",
    )
    expect(
      memoryEmbeddingErrorDetail(
        "reembed failed https://api.example.test/embeddings?api_key=query-secret Authorization: Bearer bearer-secret token=inline-secret",
      ),
    ).toBe(
      "reembed failed https://api.example.test/embeddings?api_key=[redacted] Authorization: Bearer [redacted] token=[redacted]",
    )
    expect(memoryEmbeddingErrorDetail("  timeout  ")).toBe("timeout")
    expect(memoryEmbeddingErrorDetail("   ")).toBeNull()
    expect(memoryEmbeddingErrorDetail(null)).toBeNull()
    expect(memoryEmbeddingErrorDetail(undefined)).toBeNull()
  })

  it("formats localized operation errors while preserving redacted detail", () => {
    const translations: Record<string, string> = {
      "settings.memoryEmbeddingErrors.load": "无法加载记忆向量设置",
      "settings.memoryEmbeddingErrors.reembedStart": "无法启动记忆向量重建",
      "settings.memoryEmbeddingErrors.localAssistantOpenDownload": "无法打开本地 Embedding 下载页",
      "settings.memoryEmbeddingErrors.detail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(
      memoryEmbeddingOperationErrorToast(
        "reembedStart",
        t,
        "queue is full api_key=secret-key",
      ),
    ).toEqual({
      title: "无法启动记忆向量重建",
      description: "详细信息：queue is full api_key=[redacted]",
    })
    expect(memoryEmbeddingOperationErrorToast("load", t, "config token=load-secret")).toEqual({
      title: "无法加载记忆向量设置",
      description: "详细信息：config token=[redacted]",
    })
    expect(
      memoryEmbeddingOperationErrorToast(
        "localAssistantOpenDownload",
        t,
        "browser denied Authorization: Bearer download-secret",
      ),
    ).toEqual({
      title: "无法打开本地 Embedding 下载页",
      description: "详细信息：browser denied Authorization: Bearer [redacted]",
    })
  })

  it("uses English fallback titles and omits empty detail", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(memoryEmbeddingOperationErrorToast("setDefault", t, "provider 401")).toEqual({
      title: "Failed to switch memory embedding model",
      description: "Details: provider 401",
    })
    expect(memoryEmbeddingOperationErrorToast("disable", t, "   ")).toEqual({
      title: "Failed to disable vector search",
    })
    expect(memoryEmbeddingOperationErrorToast("localAssistantOpenDownload", t, null)).toEqual({
      title: "Failed to open local embedding download page",
    })
  })

  it("formats inline assistant errors with detail on the next line", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(
      memoryEmbeddingOperationErrorText(
        "localAssistantStart",
        t,
        "ollama missing token=assistant-secret",
      ),
    ).toBe("Failed to start local embedding setup\nDetails: ollama missing token=[redacted]")
    expect(memoryEmbeddingOperationErrorText("localAssistantRefresh", t, "")).toBe(
      "Failed to refresh local embedding assistant",
    )
    expect(memoryEmbeddingOperationErrorText("localAssistantLogs", t, "db busy")).toBe(
      "Failed to load local embedding setup logs\nDetails: db busy",
    )
  })
})
