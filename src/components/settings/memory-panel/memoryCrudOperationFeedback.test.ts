import { describe, expect, it } from "vitest"

import {
  memoryCrudDiagnosticText,
  memoryCrudOperationErrorDetail,
  memoryCrudOperationErrorToast,
} from "./memoryCrudOperationFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("memory CRUD operation feedback", () => {
  it("extracts user-facing error detail", () => {
    expect(memoryCrudOperationErrorDetail(new Error("database is locked"))).toBe(
      "database is locked",
    )
    expect(memoryCrudOperationErrorDetail("  clipboard denied  ")).toBe("clipboard denied")
    expect(memoryCrudOperationErrorDetail("   ")).toBeNull()
    expect(memoryCrudOperationErrorDetail(null)).toBeNull()
    expect(memoryCrudOperationErrorDetail(undefined)).toBeNull()
    expect(
      memoryCrudDiagnosticText(
        "memory write failed https://api.example.test?token=query-secret Authorization: Bearer bearer-secret api_key=sk-live-secret",
      ),
    ).toBe(
      "memory write failed https://api.example.test?token=[redacted] Authorization: Bearer [redacted] api_key=[redacted]",
    )
    expect(
      memoryCrudOperationErrorDetail(
        "memory update failed password=db-secret passphrase=backup-secret",
      ),
    ).toBe("memory update failed password=[redacted] passphrase=[redacted]")
  })

  it("formats localized operation errors with redacted detail", () => {
    const translations: Record<string, string> = {
      "settings.memoryCrudErrors.focus": "打开记忆失败",
      "settings.memoryCrudErrors.loadAgents": "加载记忆 Agent 列表失败",
      "settings.memoryCrudErrors.loadStats": "加载记忆统计失败",
      "settings.memoryCrudErrors.pin": "置顶记忆失败",
      "settings.memoryCrudErrors.deleteBatch": "删除所选记忆失败",
      "settings.memoryCrudErrors.detail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(memoryCrudOperationErrorToast("pin", t, "permission denied")).toEqual({
      title: "置顶记忆失败",
      description: "详细信息：permission denied",
    })
    expect(memoryCrudOperationErrorToast("pin", t, "permission denied token=crud-secret")).toEqual(
      {
        title: "置顶记忆失败",
        description: "详细信息：permission denied token=[redacted]",
      },
    )
    expect(memoryCrudOperationErrorToast("focus", t, "not found")).toEqual({
      title: "打开记忆失败",
      description: "详细信息：not found",
    })
    expect(memoryCrudOperationErrorToast("deleteBatch", t, "database is locked")).toEqual({
      title: "删除所选记忆失败",
      description: "详细信息：database is locked",
    })
    expect(
      memoryCrudOperationErrorToast("loadAgents", t, "Authorization: Bearer agent-secret"),
    ).toEqual({
      title: "加载记忆 Agent 列表失败",
      description: "详细信息：Authorization: Bearer [redacted]",
    })
    expect(memoryCrudOperationErrorToast("loadStats", t, "api_key=stats-secret")).toEqual({
      title: "加载记忆统计失败",
      description: "详细信息：api_key=[redacted]",
    })
  })

  it("uses English fallback titles and omits empty detail", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(memoryCrudOperationErrorToast("checkDuplicate", t, "timeout")).toEqual({
      title: "Failed to check for similar memories",
      description: "Details: timeout",
    })
    expect(memoryCrudOperationErrorToast("loadAgents", t, "ipc unavailable")).toEqual({
      title: "Failed to load memory agent list",
      description: "Details: ipc unavailable",
    })
    expect(memoryCrudOperationErrorToast("loadStats", t, "database is locked")).toEqual({
      title: "Failed to load memory stats",
      description: "Details: database is locked",
    })
    expect(memoryCrudOperationErrorToast("export", t, "   ")).toEqual({
      title: "Failed to copy memory export",
    })
    expect(memoryCrudOperationErrorToast("delete", t, null)).toEqual({
      title: "Failed to delete memory",
    })
    expect(memoryCrudOperationErrorToast("reembedSelected", t, "embedding model missing")).toEqual({
      title: "Failed to rebuild selected memory vectors",
      description: "Details: embedding model missing",
    })
  })
})
