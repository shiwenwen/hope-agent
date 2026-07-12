import { describe, expect, it } from "vitest"

import {
  externalMemoryProviderOperationErrorDetail,
  externalMemoryProviderOperationErrorToast,
} from "./externalMemoryProviderOperationFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("external memory provider operation feedback", () => {
  it("extracts user-facing error detail", () => {
    expect(externalMemoryProviderOperationErrorDetail(new Error("config denied"))).toBe(
      "config denied",
    )
    expect(
      externalMemoryProviderOperationErrorDetail(
        "preflight failed https://api.example.test/sync?token=provider-secret Authorization: Bearer bearer-secret",
      ),
    ).toBe(
      "preflight failed https://api.example.test/sync?token=[redacted] Authorization: Bearer [redacted]",
    )
    expect(externalMemoryProviderOperationErrorDetail("  invalid provider id  ")).toBe(
      "invalid provider id",
    )
    expect(externalMemoryProviderOperationErrorDetail("   ")).toBeNull()
    expect(externalMemoryProviderOperationErrorDetail(null)).toBeNull()
    expect(externalMemoryProviderOperationErrorDetail(undefined)).toBeNull()
  })

  it("formats localized operation errors while preserving redacted diagnostic detail", () => {
    const translations: Record<string, string> = {
      "settings.memoryExternalProviderErrors.save": "保存外部记忆 Provider 失败",
      "settings.memoryExternalProviderErrors.preflight": "加载外部记忆 Provider 预检失败",
      "settings.memoryExternalProviderErrors.sync": "运行外部记忆 Provider 同步检查失败",
      "settings.memoryExternalProviderErrors.copyPreflightDiagnostics":
        "复制外部记忆 Provider 预检失败",
      "settings.memoryExternalProviderErrors.copySyncDiagnostics":
        "复制外部记忆 Provider 同步报告失败",
      "settings.memoryExternalProviderErrors.detail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(externalMemoryProviderOperationErrorToast("save", t, "policy unsupported")).toEqual({
      title: "保存外部记忆 Provider 失败",
      description: "详细信息：policy unsupported",
    })
    expect(
      externalMemoryProviderOperationErrorToast(
        "preflight",
        t,
        "sqlite locked api_key=secret-key",
      ),
    ).toEqual({
      title: "加载外部记忆 Provider 预检失败",
      description: "详细信息：sqlite locked api_key=[redacted]",
    })
    expect(
      externalMemoryProviderOperationErrorToast(
        "sync",
        t,
        "sync failed token=provider-secret",
      ),
    ).toEqual({
      title: "运行外部记忆 Provider 同步检查失败",
      description: "详细信息：sync failed token=[redacted]",
    })
    expect(
      externalMemoryProviderOperationErrorToast(
        "copyPreflightDiagnostics",
        t,
        "clipboard denied",
      ),
    ).toEqual({
      title: "复制外部记忆 Provider 预检失败",
      description: "详细信息：clipboard denied",
    })
    expect(
      externalMemoryProviderOperationErrorToast("copySyncDiagnostics", t, "clipboard denied"),
    ).toEqual({
      title: "复制外部记忆 Provider 同步报告失败",
      description: "详细信息：clipboard denied",
    })
  })

  it("uses English fallback titles and omits empty detail", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(externalMemoryProviderOperationErrorToast("load", t, "backend unavailable")).toEqual({
      title: "Failed to load external memory providers",
      description: "Details: backend unavailable",
    })
    expect(externalMemoryProviderOperationErrorToast("save", t, "   ")).toEqual({
      title: "Failed to save external memory providers",
    })
    expect(externalMemoryProviderOperationErrorToast("preflight", t, "stats unavailable")).toEqual({
      title: "Failed to load external memory provider preflight",
      description: "Details: stats unavailable",
    })
    expect(externalMemoryProviderOperationErrorToast("sync", t, "adapter unavailable")).toEqual({
      title: "Failed to run external memory provider sync check",
      description: "Details: adapter unavailable",
    })
    expect(
      externalMemoryProviderOperationErrorToast("copyPreflightDiagnostics", t, null),
    ).toEqual({
      title: "Failed to copy external memory provider preflight",
    })
    expect(externalMemoryProviderOperationErrorToast("copySyncDiagnostics", t, null)).toEqual({
      title: "Failed to copy external memory provider sync report",
    })
  })
})
