import { describe, expect, it } from "vitest"

import {
  memoryRepairDiagnosticText,
  memoryRepairOperationErrorDetail,
  memoryRepairOperationErrorToast,
} from "./memoryRepairOperationFeedback"

function interpolate(text: string, options?: Record<string, unknown>): string {
  return text.replace(/{{\s*([A-Za-z0-9_]+)\s*}}/g, (match, name: string) =>
    options && Object.prototype.hasOwnProperty.call(options, name) ? String(options[name]) : match,
  )
}

describe("memory repair operation feedback", () => {
  it("extracts user-facing error detail", () => {
    expect(memoryRepairOperationErrorDetail(new Error("database is locked"))).toBe(
      "database is locked",
    )
    expect(memoryRepairOperationErrorDetail("  quick_check failed  ")).toBe("quick_check failed")
    expect(memoryRepairOperationErrorDetail("   ")).toBeNull()
    expect(memoryRepairOperationErrorDetail(null)).toBeNull()
    expect(memoryRepairOperationErrorDetail(undefined)).toBeNull()
    expect(
      memoryRepairDiagnosticText(
        "health failed https://api.example.test?token=query-secret Authorization: Bearer bearer-secret api_key=sk-live-secret",
      ),
    ).toBe(
      "health failed https://api.example.test?token=[redacted] Authorization: Bearer [redacted] api_key=[redacted]",
    )
    expect(
      memoryRepairOperationErrorDetail(
        "snapshot restore failed password=db-secret passphrase=backup-secret",
      ),
    ).toBe("snapshot restore failed password=[redacted] passphrase=[redacted]")
  })

  it("formats localized operation errors with redacted detail", () => {
    const translations: Record<string, string> = {
      "settings.memoryRepairErrors.createDbSnapshot": "创建数据库安全快照失败",
      "settings.memoryRepairErrors.copyHealthDiagnostics": "复制记忆健康报告失败",
      "settings.memoryRepairErrors.detail": "详细信息：{{error}}",
    }
    const t = (key: string, options?: Record<string, unknown>) =>
      interpolate(translations[key] ?? String(options?.defaultValue ?? key), options)

    expect(memoryRepairOperationErrorToast("createDbSnapshot", t, "permission denied")).toEqual({
      title: "创建数据库安全快照失败",
      description: "详细信息：permission denied",
    })
    expect(
      memoryRepairOperationErrorToast("createDbSnapshot", t, "permission denied token=repair-secret"),
    ).toEqual({
      title: "创建数据库安全快照失败",
      description: "详细信息：permission denied token=[redacted]",
    })
    expect(memoryRepairOperationErrorToast("copyHealthDiagnostics", t, "clipboard denied")).toEqual(
      {
        title: "复制记忆健康报告失败",
        description: "详细信息：clipboard denied",
      },
    )
  })

  it("uses English fallback titles and omits empty detail", () => {
    const t = (_key: string, options?: Record<string, unknown>) =>
      interpolate(String(options?.defaultValue ?? ""), options)

    expect(memoryRepairOperationErrorToast("rebuildFts", t, "writer busy")).toEqual({
      title: "Failed to rebuild keyword index",
      description: "Details: writer busy",
    })
    expect(memoryRepairOperationErrorToast("restorePreview", t, "   ")).toEqual({
      title: "Snapshot restore preflight failed",
    })
    expect(
      memoryRepairOperationErrorToast("copyDbSnapshotVerification", t, "clipboard denied"),
    ).toEqual({
      title: "Failed to copy snapshot verification",
      description: "Details: clipboard denied",
    })
    expect(memoryRepairOperationErrorToast("loadHealth", t, "database locked")).toEqual({
      title: "Failed to load memory health",
      description: "Details: database locked",
    })
    expect(memoryRepairOperationErrorToast("copyRestorePreview", t, "clipboard denied")).toEqual({
      title: "Failed to copy restore preflight report",
      description: "Details: clipboard denied",
    })
    expect(
      memoryRepairOperationErrorToast("restoreDbSnapshot", t, new Error("checksum mismatch")),
    ).toEqual({
      title: "Failed to restore database snapshot",
      description: "Details: checksum mismatch",
    })
  })
})
