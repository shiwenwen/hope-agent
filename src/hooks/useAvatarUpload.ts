import { useEffect, useRef, useState } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"

export interface UseAvatarUploadOptions {
  /**
   * Produce the filename to pass to `save_avatar`. Called once per crop
   * confirmation; the blob is supplied so callers can inspect its mime
   * if they want an extension other than `.png`.
   */
  fileName: (blob: Blob) => string
  /** Prefix for structured log entries on pick / save failures. */
  logCategory: string
  /** Invoked with the absolute on-disk path returned by `save_avatar`. */
  onSaved: (path: string) => void
}

export interface UseAvatarUploadResult {
  cropSrc: string | null
  handleAvatarPick: () => Promise<void>
  handleCropCancel: () => void
  handleCropConfirm: (blob: Blob) => Promise<void>
}

/**
 * Shared "pick local image → crop → upload → persist path" flow behind
 * both the user profile avatar and the agent avatar UI. Keeps Blob URL
 * lifetimes correct — a single Blob URL may be live between
 * `handleAvatarPick` returning and the crop dialog closing (confirm or
 * cancel), and must also be revoked on unmount if the caller unmounts
 * while the dialog is open.
 */
export function useAvatarUpload(opts: UseAvatarUploadOptions): UseAvatarUploadResult {
  const [cropSrc, setCropSrc] = useState<string | null>(null)
  const pendingRevokeRef = useRef<(() => void) | null>(null)

  useEffect(() => {
    return () => {
      if (pendingRevokeRef.current) pendingRevokeRef.current()
    }
  }, [])

  const clearPendingRevoke = () => {
    if (pendingRevokeRef.current) {
      pendingRevokeRef.current()
      pendingRevokeRef.current = null
    }
  }

  const handleAvatarPick = async () => {
    try {
      const picked = await getTransport().pickLocalImage()
      if (!picked) return
      clearPendingRevoke()
      pendingRevokeRef.current = picked.revoke ?? null
      setCropSrc(picked.src)
    } catch (e) {
      logger.error("settings", `${opts.logCategory}::pickAvatar`, "Failed to pick avatar", e)
    }
  }

  const handleCropCancel = () => {
    setCropSrc(null)
    clearPendingRevoke()
  }

  const handleCropConfirm = async (blob: Blob) => {
    setCropSrc(null)
    clearPendingRevoke()
    try {
      const buf = await blob.arrayBuffer()
      const transport = getTransport()
      const savedPath = await transport.call<string>("save_avatar", {
        data: transport.prepareFileData(buf, blob.type || "image/png"),
        fileName: opts.fileName(blob),
      })
      opts.onSaved(savedPath)
    } catch (e) {
      logger.error("settings", `${opts.logCategory}::saveAvatar`, "Failed to save avatar", e)
    }
  }

  return { cropSrc, handleAvatarPick, handleCropCancel, handleCropConfirm }
}
