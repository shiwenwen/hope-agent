/**
 * Unified file-operation policy shared by every place a file appears in chat
 * (Markdown links, message attachments, the workspace panel). Pure logic only —
 * no transport, no React — so it is trivially testable. The actual dispatch of
 * an action to transport methods lives in `useFileActions`.
 *
 * Behavior matrix (driven by `isLocal = transport.supportsLocalFileOps()`):
 *
 *   | kind         | local (desktop)        | remote (HTTP/Web)   |
 *   | ------------ | ---------------------- | ------------------- |
 *   | previewable  | click → preview        | click → preview     |
 *   | other        | click → open (OS app)  | click → download    |
 *
 *   menu (local):  [preview?, open, reveal-in-folder]
 *   menu (remote): [preview?, download]
 */

import { Download, ExternalLink, Eye, FolderOpen, type LucideIcon } from "lucide-react"
import { isPreviewableKind, type FileKind } from "./fileKind"

export type FileAction = "preview" | "open" | "download" | "reveal"

/** The single action a primary (left) click performs. */
export function resolvePrimaryFileAction(
  kind: FileKind,
  isLocal: boolean,
): Exclude<FileAction, "reveal"> {
  if (isPreviewableKind(kind)) return "preview"
  return isLocal ? "open" : "download"
}

/** Ordered actions for the right-click / "⋯ more" menu. */
export function resolveFileMenuActions(kind: FileKind, isLocal: boolean): FileAction[] {
  const actions: FileAction[] = []
  if (isPreviewableKind(kind)) actions.push("preview")
  if (isLocal) {
    actions.push("open", "reveal")
  } else {
    actions.push("download")
  }
  return actions
}

/** i18n key + fallback label + icon for each action (UI rendering metadata). */
export const FILE_ACTION_META: Record<
  FileAction,
  { labelKey: string; defaultLabel: string; icon: LucideIcon }
> = {
  preview: { labelKey: "fileActions.preview", defaultLabel: "Preview", icon: Eye },
  open: { labelKey: "fileActions.open", defaultLabel: "Open", icon: ExternalLink },
  download: { labelKey: "fileActions.download", defaultLabel: "Download", icon: Download },
  reveal: { labelKey: "fileActions.revealInFolder", defaultLabel: "Reveal in folder", icon: FolderOpen },
}
