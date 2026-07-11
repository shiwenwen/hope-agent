import type { MemoryBackupImportPreview } from "./types"

export type MemoryBackupUnlockTranslateFn = (key: string, defaultValue: string) => string

export interface MemoryBackupUnlockFailureToast {
  title: string
  description?: string
}

export function shouldKeepMemoryBackupUnlockDialogOpen(
  preview: Pick<MemoryBackupImportPreview, "issues"> | null | undefined,
): boolean {
  return preview?.issues.some((issue) => issue.code === "encrypted_decrypt_failed") ?? false
}

export function shouldOpenMemoryBackupPreviewAfterUnlockFailure(
  preview: Pick<MemoryBackupImportPreview, "valid" | "issues"> | null | undefined,
): boolean {
  return Boolean(preview && !preview.valid && !shouldKeepMemoryBackupUnlockDialogOpen(preview))
}

export function memoryBackupUnlockFailureToast(
  preview: Pick<MemoryBackupImportPreview, "valid" | "issues"> | null | undefined,
  t: MemoryBackupUnlockTranslateFn,
): MemoryBackupUnlockFailureToast {
  if (shouldKeepMemoryBackupUnlockDialogOpen(preview)) {
    return {
      title: t("settings.memoryBackupEncryptedDecryptFailed", "Could not decrypt this backup"),
      description: t(
        "settings.memoryBackupEncryptedDecryptRetry",
        "Check the passphrase and try again. The unlock dialog will stay open.",
      ),
    }
  }

  if (shouldOpenMemoryBackupPreviewAfterUnlockFailure(preview)) {
    return {
      title: t("settings.memoryBackupPreviewInvalid", "This backup cannot be imported"),
      description: t(
        "settings.memoryBackupEncryptedDiagnosticsOpened",
        "Preview diagnostics opened with the backup errors.",
      ),
    }
  }

  return {
    title: t("settings.memoryBackupPreviewInvalid", "This backup cannot be imported"),
  }
}
