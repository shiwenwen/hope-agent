import { useEffect, useState } from "react"
import { TRANSPORT_EVENT_RESYNC_REQUIRED, type Transport } from "@/lib/transport"
import { useTransport } from "@/lib/transport-provider"

export const DEFAULT_MAX_CHAT_ATTACHMENT_MB = 20
export const MIN_MAX_CHAT_ATTACHMENT_MB = 1
export const MAX_MAX_CHAT_ATTACHMENT_MB = 512
export const DEFAULT_MAX_WORKSPACE_UPLOAD_MB = 20
export const MIN_MAX_WORKSPACE_UPLOAD_MB = 1
export const MAX_MAX_WORKSPACE_UPLOAD_MB = 512
export const DEFAULT_MAX_TEXT_PREVIEW_MB = 5
export const MIN_MAX_TEXT_PREVIEW_MB = 1
export const MAX_MAX_TEXT_PREVIEW_MB = 50
export const DEFAULT_MAX_TEXT_EDIT_MB = 5
export const MIN_MAX_TEXT_EDIT_MB = 1
export const MAX_MAX_TEXT_EDIT_MB = 20
export const DEFAULT_MAX_DOCUMENT_PREVIEW_MB = 50
export const MIN_MAX_DOCUMENT_PREVIEW_MB = 5
export const MAX_MAX_DOCUMENT_PREVIEW_MB = 100
export const MEBIBYTE_BYTES = 1024 * 1024

export interface FilesystemConfig {
  allowRemoteWrites: boolean
  maxChatAttachmentMb: number
  maxWorkspaceUploadMb: number
  maxTextPreviewMb: number
  maxTextEditMb: number
  maxDocumentPreviewMb: number
}

export const DEFAULT_FILESYSTEM_CONFIG: FilesystemConfig = {
  allowRemoteWrites: false,
  maxChatAttachmentMb: DEFAULT_MAX_CHAT_ATTACHMENT_MB,
  maxWorkspaceUploadMb: DEFAULT_MAX_WORKSPACE_UPLOAD_MB,
  maxTextPreviewMb: DEFAULT_MAX_TEXT_PREVIEW_MB,
  maxTextEditMb: DEFAULT_MAX_TEXT_EDIT_MB,
  maxDocumentPreviewMb: DEFAULT_MAX_DOCUMENT_PREVIEW_MB,
}

function normalizedLimit(value: unknown, fallback: number, min: number, max: number): number {
  const raw = Number(value)
  const rounded = Number.isFinite(raw) ? Math.round(raw) : fallback
  return Math.min(max, Math.max(min, rounded))
}

export function normalizeFilesystemConfig(
  value?: Partial<FilesystemConfig> | null,
): FilesystemConfig {
  const maxTextPreviewMb = normalizedLimit(
    value?.maxTextPreviewMb,
    DEFAULT_MAX_TEXT_PREVIEW_MB,
    MIN_MAX_TEXT_PREVIEW_MB,
    MAX_MAX_TEXT_PREVIEW_MB,
  )
  return {
    allowRemoteWrites: value?.allowRemoteWrites === true,
    maxChatAttachmentMb: normalizedLimit(
      value?.maxChatAttachmentMb,
      DEFAULT_MAX_CHAT_ATTACHMENT_MB,
      MIN_MAX_CHAT_ATTACHMENT_MB,
      MAX_MAX_CHAT_ATTACHMENT_MB,
    ),
    maxWorkspaceUploadMb: normalizedLimit(
      value?.maxWorkspaceUploadMb,
      DEFAULT_MAX_WORKSPACE_UPLOAD_MB,
      MIN_MAX_WORKSPACE_UPLOAD_MB,
      MAX_MAX_WORKSPACE_UPLOAD_MB,
    ),
    maxTextPreviewMb,
    maxTextEditMb: Math.min(
      maxTextPreviewMb,
      normalizedLimit(
        value?.maxTextEditMb,
        DEFAULT_MAX_TEXT_EDIT_MB,
        MIN_MAX_TEXT_EDIT_MB,
        MAX_MAX_TEXT_EDIT_MB,
      ),
    ),
    maxDocumentPreviewMb: normalizedLimit(
      value?.maxDocumentPreviewMb,
      DEFAULT_MAX_DOCUMENT_PREVIEW_MB,
      MIN_MAX_DOCUMENT_PREVIEW_MB,
      MAX_MAX_DOCUMENT_PREVIEW_MB,
    ),
  }
}

export function maxChatAttachmentBytes(config: FilesystemConfig): number {
  return normalizeFilesystemConfig(config).maxChatAttachmentMb * MEBIBYTE_BYTES
}

export async function readFilesystemConfig(transport: Transport): Promise<FilesystemConfig> {
  const value = await transport.call<Partial<FilesystemConfig>>("get_filesystem_config")
  return normalizeFilesystemConfig(value)
}

export async function patchFilesystemConfig(
  transport: Transport,
  patch: Partial<FilesystemConfig>,
): Promise<FilesystemConfig> {
  const value = await transport.call<Partial<FilesystemConfig>>("patch_filesystem_config", {
    patch,
  })
  return normalizeFilesystemConfig(value)
}

export function useFilesystemConfig(): {
  config: FilesystemConfig
  loading: boolean
  refresh: () => Promise<void>
} {
  const transport = useTransport()
  const [config, setConfig] = useState(DEFAULT_FILESYSTEM_CONFIG)
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    let cancelled = false
    const refresh = async () => {
      try {
        const next = await readFilesystemConfig(transport)
        if (!cancelled) setConfig(next)
      } catch {
        // Keep the last known/default value while a remote transport is
        // reconnecting. The resync event below retries automatically.
      } finally {
        if (!cancelled) setLoading(false)
      }
    }
    void refresh()
    const unlistenConfig = transport.listen("config:changed", () => void refresh())
    const unlistenReconnect = transport.listen(TRANSPORT_EVENT_RESYNC_REQUIRED, () => void refresh())
    return () => {
      cancelled = true
      unlistenConfig()
      unlistenReconnect()
    }
  }, [transport])

  return {
    config,
    loading,
    refresh: async () => setConfig(await readFilesystemConfig(transport)),
  }
}
