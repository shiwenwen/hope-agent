import { useCallback, useMemo } from "react"

import { toast } from "sonner"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { fileKindOf, type FileKind } from "@/lib/fileKind"
import {
  resolveFileMenuActions,
  resolvePrimaryFileAction,
  type FileAction,
} from "@/lib/fileActions"
import { useFileActionsContext } from "./fileActionsContext"
import type { PreviewTarget } from "./useFilePreview"

export interface FileActionsOverrides {
  /** Override the ambient session id (e.g. the workspace panel passes its own). */
  sessionId?: string | null
  /** Override the ambient preview opener (panels outside the message tree). */
  onPreviewFile?: (target: PreviewTarget) => void
}

export interface FileActionsResult {
  kind: FileKind
  /** Action a primary (left) click performs. */
  primary: FileAction
  /** Ordered actions for the right-click / "⋯ more" menu. */
  menu: FileAction[]
  isLocal: boolean
  /** Whether a preview panel is wired (otherwise preview is dropped). */
  canPreview: boolean
  /** Dispatch an action to the transport / preview panel. */
  run: (action: FileAction) => void
}

function logFail(action: string, e: unknown) {
  logger.error("chat", `useFileActions::${action}`, "file action failed", e)
  // Surface a user-visible error (open/download/reveal otherwise fail silently;
  // preview failures are shown inside the preview panel itself).
  toast.error(e instanceof Error ? e.message : String(e))
}

/**
 * Resolve + dispatch the unified file operations for a single target. Reads
 * `sessionId` / `onPreviewFile` from {@link useFileActionsContext}; callers
 * outside the message tree (the workspace panel) pass `overrides`.
 *
 * `target` may be `null` (e.g. a Markdown link that isn't a local file) — the
 * result is then inert (`menu: []`, `run` no-ops) so the hook stays
 * unconditional.
 */
export function useFileActions(
  target: PreviewTarget | null,
  overrides?: FileActionsOverrides,
): FileActionsResult {
  const ctx = useFileActionsContext()
  const sessionId = overrides?.sessionId ?? ctx.sessionId
  const onPreviewFile = overrides?.onPreviewFile ?? ctx.onPreviewFile
  const transport = getTransport()
  const isLocal = transport.supportsLocalFileOps()
  const canPreview = !!onPreviewFile

  const kind = useMemo<FileKind>(() => {
    if (!target) return "other"
    return target.kind === "media"
      ? fileKindOf(target.item.name, target.item.mimeType)
      : fileKindOf(target.name, target.mime, target.language)
  }, [target])

  const primary = useMemo<FileAction>(() => {
    const p = resolvePrimaryFileAction(kind, isLocal)
    // No preview panel wired → fall back to the mode's non-preview default.
    return p === "preview" && !canPreview ? (isLocal ? "open" : "download") : p
  }, [kind, isLocal, canPreview])

  const menu = useMemo<FileAction[]>(() => {
    if (!target) return []
    const actions = resolveFileMenuActions(kind, isLocal)
    return canPreview ? actions : actions.filter((a) => a !== "preview")
  }, [target, kind, isLocal, canPreview])

  const run = useCallback(
    (action: FileAction) => {
      if (!target) return
      switch (action) {
        case "preview":
          onPreviewFile?.(target)
          break
        case "open":
          if (target.kind === "media") {
            void transport.openMedia(target.item).catch((e) => logFail("open", e))
          } else {
            void transport.openFilePath(target.path, { sessionId }).catch((e) => logFail("open", e))
          }
          break
        case "download":
          if (target.kind === "media") {
            void transport.downloadMedia(target.item).catch((e) => logFail("download", e))
          } else {
            void transport
              .downloadFilePath(target.path, { sessionId, filename: target.name })
              .catch((e) => logFail("download", e))
          }
          break
        case "reveal":
          if (target.kind === "media") {
            void transport.revealMedia(target.item).catch((e) => logFail("reveal", e))
          } else {
            void transport
              .call("reveal_in_folder", { path: target.path })
              .catch((e) => logFail("reveal", e))
          }
          break
      }
    },
    [target, onPreviewFile, sessionId, transport],
  )

  return { kind, primary, menu, isLocal, canPreview, run }
}
