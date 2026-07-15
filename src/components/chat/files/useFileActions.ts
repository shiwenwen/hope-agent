import { useCallback, useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"

import { toast } from "sonner"
import { useTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { FileKind } from "@/lib/fileKind"
import type { WorkspaceAccess } from "@/lib/transport"
import type { FileAction } from "@/lib/fileActions"
import { useFileActionsContext } from "./fileActionsContext"
import { fileTargetKind, resolveFileCapabilities } from "./fileCapabilities"
import { fileResourceAdapterFor } from "./fileResourceAdapter"
import type { FileActionInput, FileActionRunResult, FileCapabilitySet } from "./types"
import type { WorkspaceFileOperations } from "./fileResourceAdapter"
import type { PreviewTarget } from "./useFilePreview"
import { MEBIBYTE_BYTES, useFilesystemConfig } from "@/lib/filesystemConfig"

export interface FileActionsOverrides {
  /** Override the ambient session id (e.g. the workspace panel passes its own). */
  sessionId?: string | null
  /** Override the ambient preview opener (panels outside the message tree). */
  onPreviewFile?: (target: PreviewTarget) => void
  onEditFile?: (target: PreviewTarget) => void
  onRemoveFile?: (target: PreviewTarget) => void
  onGuidedAction?: (action: FileAction, target: PreviewTarget) => void
  workspaceAccess?: WorkspaceAccess
  workspaceOperations?: WorkspaceFileOperations
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
  capabilities: FileCapabilitySet
  /** Dispatch an action to the transport / preview panel. */
  run: (action: FileAction, input?: FileActionInput) => Promise<FileActionRunResult>
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
  const { t } = useTranslation()
  const sessionId = overrides?.sessionId ?? ctx.sessionId
  const onPreviewFile = overrides?.onPreviewFile ?? ctx.onPreviewFile
  const transport = useTransport()
  const { config: filesystemConfig } = useFilesystemConfig()
  const runtime = transport.fileRuntime()
  const isLocal = runtime.workspaceHost === "local"
  const canPreview = !!onPreviewFile
  const [workspaceAccess, setWorkspaceAccess] = useState<WorkspaceAccess | undefined>()

  useEffect(() => {
    if (target?.kind !== "workspace" || overrides?.workspaceAccess) {
      return
    }
    let cancelled = false
    setWorkspaceAccess(undefined)
    void transport
      .getWorkspaceAccess({ scope: target.scope, scopeId: target.scopeId })
      .then((access) => {
        if (!cancelled) setWorkspaceAccess(access)
      })
      .catch((error) => {
        if (!cancelled) logFail("capabilities", error)
      })
    return () => {
      cancelled = true
    }
  }, [target, transport, overrides?.workspaceAccess])

  const effectiveWorkspaceAccess = overrides?.workspaceAccess ?? workspaceAccess

  const kind = useMemo<FileKind>(() => {
    return target ? fileTargetKind(target) : "other"
  }, [target])

  const capabilities = useMemo(() => {
    const resolved = target
      ? resolveFileCapabilities(
          target,
          runtime,
          effectiveWorkspaceAccess,
          filesystemConfig.maxTextEditMb * MEBIBYTE_BYTES,
        )
      : resolveFileCapabilities(
          {
            kind: "sessionPath",
            path: "",
            name: "",
          },
          runtime,
          undefined,
          filesystemConfig.maxTextEditMb * MEBIBYTE_BYTES,
        )
    return {
      ...resolved,
      ...(!overrides?.onEditFile && {
        edit: { state: "disabled" as const, reason: "not_supported" as const },
      }),
      ...(!overrides?.onRemoveFile && {
        remove: { state: "disabled" as const, reason: "not_supported" as const },
      }),
      ...(!overrides?.onRemoveFile &&
        !overrides?.workspaceOperations && {
          delete: { state: "disabled" as const, reason: "not_supported" as const },
        }),
    }
  }, [
    target,
    runtime,
    effectiveWorkspaceAccess,
    filesystemConfig.maxTextEditMb,
    overrides?.onEditFile,
    overrides?.onRemoveFile,
    overrides?.workspaceOperations,
  ])

  const primary = useMemo<FileAction>(() => {
    if (target && canPreview && capabilities.preview.state === "enabled") return "preview"
    return isLocal ? "open" : "download"
  }, [target, isLocal, canPreview, capabilities.preview.state])

  const menu = useMemo<FileAction[]>(() => {
    if (!target) return []
    const actions: FileAction[] = canPreview ? ["preview"] : []
    if (target.kind === "clientDraft") {
      actions.push("open", "download", "edit", "remove")
    } else if (target.kind === "workspace") {
      actions.push("edit", "open", ...(isLocal ? (["reveal"] as const) : (["download"] as const)))
    } else if (target.kind === "knowledgeNote") {
      actions.push("edit", "open", "download")
    } else if (isLocal) {
      actions.push("open", "reveal")
    } else {
      actions.push("open", "download")
    }
    return actions.filter((action) => capabilities[action].state !== "disabled")
  }, [target, isLocal, canPreview, capabilities])

  const run = useCallback(
    async (action: FileAction, input?: FileActionInput): Promise<FileActionRunResult> => {
      if (!target) return "disabled"
      const capability = capabilities[action]
      if (capability.state === "guided") {
        if (overrides?.onGuidedAction) overrides.onGuidedAction(action, target)
        else toast.info(t("fileEditor.remoteWritesTitle", "Remote file writes are off"))
        return "guided"
      }
      if (capability.state === "disabled") return "disabled"
      if (input?.prepareOnly) return "executed"
      try {
        const executed = await fileResourceAdapterFor(target).run(
          target,
          action,
          {
            transport,
            sessionId,
            workspaceAccess: effectiveWorkspaceAccess,
            filesystemConfig,
            onPreview: onPreviewFile,
            onEdit: overrides?.onEditFile,
            onRemove: overrides?.onRemoveFile,
            workspaceOperations: overrides?.workspaceOperations,
          },
          input,
        )
        return executed ? "executed" : "failed"
      } catch (error) {
        logFail(action, error)
        return "failed"
      }
    },
    [
      capabilities,
      target,
      onPreviewFile,
      overrides,
      sessionId,
      t,
      transport,
      effectiveWorkspaceAccess,
      filesystemConfig,
    ],
  )

  return { kind, primary, menu, isLocal, canPreview, capabilities, run }
}
