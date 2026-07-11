import { describe, expect, it } from "vitest"

import {
  memoryBackupUnlockFailureToast,
  shouldKeepMemoryBackupUnlockDialogOpen,
  shouldOpenMemoryBackupPreviewAfterUnlockFailure,
} from "./memoryBackupUnlockFlow"

function invalidPreview(codes: string[]) {
  return {
    valid: false,
    issues: codes.map((code) => ({
      severity: "error" as const,
      code,
      message: code,
    })),
  }
}

describe("memory backup unlock flow", () => {
  const t = (key: string, fallback: string) => {
    const translations: Record<string, string> = {
      "settings.memoryBackupEncryptedDecryptFailed": "无法解密此备份",
      "settings.memoryBackupEncryptedDecryptRetry": "请检查口令后重试；解锁弹窗会保持打开。",
      "settings.memoryBackupPreviewInvalid": "此备份无法导入",
      "settings.memoryBackupEncryptedDiagnosticsOpened": "预览诊断已打开，里面列出了备份错误。",
    }
    return translations[key] ?? fallback
  }

  it("keeps the unlock dialog open for wrong passphrases so the user can retry", () => {
    const preview = invalidPreview(["encrypted_decrypt_failed"])

    expect(shouldKeepMemoryBackupUnlockDialogOpen(preview)).toBe(true)
    expect(shouldOpenMemoryBackupPreviewAfterUnlockFailure(preview)).toBe(false)
    expect(memoryBackupUnlockFailureToast(preview, t)).toEqual({
      title: "无法解密此备份",
      description: "请检查口令后重试；解锁弹窗会保持打开。",
    })
  })

  it("opens the preview diagnostics for non-retryable encrypted backup failures", () => {
    const preview = invalidPreview(["encrypted_plaintext_invalid"])

    expect(shouldKeepMemoryBackupUnlockDialogOpen(preview)).toBe(false)
    expect(shouldOpenMemoryBackupPreviewAfterUnlockFailure(preview)).toBe(true)
    expect(memoryBackupUnlockFailureToast(preview, t)).toEqual({
      title: "此备份无法导入",
      description: "预览诊断已打开，里面列出了备份错误。",
    })
  })

  it("does not open a failure preview for valid backups", () => {
    const preview = {
      valid: true,
      issues: [],
    }

    expect(shouldKeepMemoryBackupUnlockDialogOpen(preview)).toBe(false)
    expect(shouldOpenMemoryBackupPreviewAfterUnlockFailure(preview)).toBe(false)
    expect(memoryBackupUnlockFailureToast(preview, t)).toEqual({
      title: "此备份无法导入",
    })
  })
})
