import { describe, expect, it } from "vitest"

import {
  memoryBackupDiagnosticText,
  memoryBackupOperationErrorDetail,
  memoryBackupOperationErrorToast,
} from "./memoryBackupOperationFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("memory backup operation feedback", () => {
  it("redacts sensitive backup diagnostics", () => {
    const diagnostic = memoryBackupDiagnosticText(
      "preview failed passphrase=backup-secret password: other-secret Authorization: Bearer bearer-secret api_key=sk-live-secret",
    )

    expect(diagnostic).toContain("passphrase=[redacted]")
    expect(diagnostic).toContain("password: [redacted]")
    expect(diagnostic).toContain("Authorization: Bearer [redacted]")
    expect(diagnostic).toContain("api_key=[redacted]")
    expect(diagnostic).not.toContain("backup-secret")
    expect(diagnostic).not.toContain("other-secret")
    expect(diagnostic).not.toContain("bearer-secret")
    expect(diagnostic).not.toContain("sk-live-secret")
  })

  it("extracts user-facing error detail", () => {
    expect(memoryBackupOperationErrorDetail(new Error("permission denied"))).toBe(
      "permission denied",
    )
    expect(memoryBackupOperationErrorDetail("invalid archive token=restore-secret")).toBe(
      "invalid archive token=[redacted]",
    )
    expect(memoryBackupOperationErrorDetail("  invalid archive  ")).toBe("invalid archive")
    expect(memoryBackupOperationErrorDetail("   ")).toBeNull()
    expect(memoryBackupOperationErrorDetail(null)).toBeNull()
    expect(memoryBackupOperationErrorDetail(undefined)).toBeNull()
  })

  it("formats localized operation errors while preserving redacted detail", () => {
    const translations: Record<string, string> = {
      "settings.memoryBackupErrors.preview": "预览记忆备份失败",
      "settings.memoryBackupErrors.copyPreviewDiagnostics": "复制备份预览诊断失败",
      "settings.memoryBackupErrors.detail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(
      memoryBackupOperationErrorToast(
        "preview",
        t,
        "schema version is unsupported passphrase=secret-passphrase",
      ),
    ).toEqual({
      title: "预览记忆备份失败",
      description: "详细信息：schema version is unsupported passphrase=[redacted]",
    })
    expect(
      memoryBackupOperationErrorToast("copyPreviewDiagnostics", t, "clipboard denied"),
    ).toEqual({
      title: "复制备份预览诊断失败",
      description: "详细信息：clipboard denied",
    })
  })

  it("uses English fallback titles and omits empty detail", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(memoryBackupOperationErrorToast("export", t, "disk full")).toEqual({
      title: "Failed to export memory backup",
      description: "Details: disk full",
    })
    expect(memoryBackupOperationErrorToast("restoreLegacy", t, "   ")).toEqual({
      title: "Failed to restore memory backup",
    })
    expect(memoryBackupOperationErrorToast("copyPreviewDiagnostics", t, null)).toEqual({
      title: "Failed to copy backup preview diagnostics",
    })
    expect(
      memoryBackupOperationErrorToast("restoreStructured", t, new Error("claim conflict")),
    ).toEqual({
      title: "Failed to restore structured memory",
      description: "Details: claim conflict",
    })
  })
})
