/**
 * Recursive, lazily-loaded file tree for a project/session workspace. Renders
 * one directory level at a time (children load on expand) and supports the full
 * file-management surface: context menu, inline rename / create, drag-drop
 * upload, and delete confirmation. Pure view over a {@link ProjectFsApi}.
 */

import {
  createElement,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type DragEvent,
  type KeyboardEvent,
} from "react"
import { useTranslation } from "react-i18next"
import { ChevronRight, Loader2 } from "lucide-react"
import { toast } from "sonner"

import { cn } from "@/lib/utils"
import { iconForEntry } from "@/lib/fileKind"
import { FileTypeIcon } from "@/components/icons/FileTypeIcon"
import { AnimatedCollapse } from "@/components/ui/animated-presence"
import { Input } from "@/components/ui/input"
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import type { WorkspaceEntry } from "@/lib/transport"
import type { ProjectFsApi } from "../hooks/useProjectFs"
import type { UseTreeExpansion } from "../hooks/useTreeExpansion"
import { useFileResource } from "@/components/chat/files/useFileResource"
import { FILE_ACTION_META } from "@/lib/fileActions"
import type { PreviewTarget } from "@/components/chat/files/useFilePreview"

export interface DraftNode {
  /** Parent directory the draft is being created in. */
  dir: string
  isDir: boolean
}

interface TreeContext {
  fs: ProjectFsApi
  expansion: UseTreeExpansion
  selectedPath: string | null
  onSelectFile: (entry: WorkspaceEntry) => void
  editable: boolean
  renaming: string | null
  setRenaming: (p: string | null) => void
  draft: DraftNode | null
  setDraft: (d: DraftNode | null) => void
  dragOverDir: string | null
  setDragOverDir: (d: string | null) => void
  requestDelete: (entry: WorkspaceEntry) => void
  onCreated?: (entry: WorkspaceEntry) => void
  onGuidedWrite?: () => void
  onEditFile?: (entry: WorkspaceEntry) => void
}

function parentDir(rel: string): string {
  const i = rel.lastIndexOf("/")
  return i >= 0 ? rel.slice(0, i) : ""
}

function joinRel(dir: string, name: string): string {
  return dir ? `${dir}/${name}` : name
}

const ROW = "flex items-center gap-1 rounded-md px-1.5 py-1 text-sm cursor-pointer select-none"
const INDENT = 12

export interface FileBrowserTreeProps {
  fs: ProjectFsApi
  expansion: UseTreeExpansion
  selectedPath: string | null
  onSelectFile: (entry: WorkspaceEntry) => void
  editable?: boolean
  /** Draft "new file/folder" row, owned by the parent so the toolbar and the
   *  context menu can both trigger it. */
  draft: DraftNode | null
  onDraftChange: (draft: DraftNode | null) => void
  onCreated?: (entry: WorkspaceEntry) => void
  onGuidedWrite?: () => void
  onEditFile?: (entry: WorkspaceEntry) => void
  className?: string
}

export function FileBrowserTree({
  fs,
  expansion,
  selectedPath,
  onSelectFile,
  editable = false,
  draft,
  onDraftChange,
  onCreated,
  onGuidedWrite,
  onEditFile,
  className,
}: FileBrowserTreeProps) {
  const { t } = useTranslation()
  const [renaming, setRenaming] = useState<string | null>(null)
  const [dragOverDir, setDragOverDir] = useState<string | null>(null)
  const [deleteTarget, setDeleteTarget] = useState<WorkspaceEntry | null>(null)
  const rootTarget = useMemo<PreviewTarget>(
    () => ({
      kind: "workspace",
      scope: fs.scope.scope,
      scopeId: fs.scope.scopeId,
      relPath: "",
      name: "",
      isDirectory: true,
    }),
    [fs.scope.scope, fs.scope.scopeId],
  )
  const rootActions = useFileResource(rootTarget, {
    workspaceAccess: fs.access ?? undefined,
    workspaceOperations: fs,
    onGuidedAction: () => onGuidedWrite?.(),
  })
  const deleteTargetResource = useMemo<PreviewTarget | null>(
    () =>
      deleteTarget
        ? {
            kind: "workspace",
            scope: fs.scope.scope,
            scopeId: fs.scope.scopeId,
            relPath: deleteTarget.relPath,
            name: deleteTarget.name,
            sizeBytes: deleteTarget.size,
            isDirectory: deleteTarget.isDir,
          }
        : null,
    [deleteTarget, fs.scope.scope, fs.scope.scopeId],
  )
  const deleteActions = useFileResource(deleteTargetResource, {
    workspaceAccess: fs.access ?? undefined,
    workspaceOperations: fs,
    onGuidedAction: () => onGuidedWrite?.(),
  })

  // Load the root level once.
  const rootState = fs.getDir("")
  useEffect(() => {
    if (!rootState && fs.available) void fs.loadDir("")
  }, [rootState, fs])

  // Reveal support: when the selection is set externally (a composer quote-chip
  // click) onto a collapsed path, expand its ancestor directories so the row
  // gets rendered + highlighted. This runs in an effect (not render) so the
  // localStorage write inside setOpen stays out of render, and `expansion` is
  // already the active (host) scope. The selected row scrolls itself into view
  // once mounted (see TreeNode).
  useEffect(() => {
    if (!selectedPath) return
    const parts = selectedPath.split("/").filter(Boolean)
    parts.pop() // ancestors only — drop the file name
    let dir = ""
    for (const part of parts) {
      dir = dir ? `${dir}/${part}` : part
      expansion.setOpen(dir, true)
      // Proactively load each ancestor's listing so the target row renders
      // promptly, instead of waiting for the per-node load-on-expand to cascade
      // level by level (which can stall before reaching a deep target).
      if (!fs.getDir(dir)) void fs.loadDir(dir)
    }
  }, [selectedPath, expansion, fs])

  const ctx: TreeContext = {
    fs,
    expansion,
    selectedPath,
    onSelectFile,
    editable,
    renaming,
    setRenaming,
    draft,
    setDraft: onDraftChange,
    dragOverDir,
    setDragOverDir,
    requestDelete: setDeleteTarget,
    onCreated,
    onGuidedWrite,
    onEditFile,
  }

  const handleRootDrop = useCallback(
    async (e: DragEvent) => {
      e.preventDefault()
      e.stopPropagation()
      setDragOverDir(null)
      const files = Array.from(e.dataTransfer.files)
      if (!editable || files.length === 0) return
      const result = await rootActions.run("upload", { dirPath: "", files })
      if (result === "failed") toast.error(t("fileBrowser.uploadFailed", "Upload failed"))
    },
    [editable, rootActions, t],
  )

  const confirmDelete = useCallback(async () => {
    const target = deleteTarget
    setDeleteTarget(null)
    if (!target) return
    const result = await deleteActions.run("delete", {
      path: target.relPath,
      recursive: target.isDir,
    })
    if (result === "failed") toast.error(t("fileBrowser.deleteFailed", "Delete failed"))
  }, [deleteActions, deleteTarget, t])

  const entries = rootState?.entries ?? []

  return (
    <>
      <div
        className={cn(
          "min-h-full py-1",
          editable && dragOverDir === "" && "bg-accent/40",
          className,
        )}
        onDragOver={(e) => {
          if (!editable) return
          e.preventDefault()
          setDragOverDir("")
        }}
        onDragLeave={() => setDragOverDir((d) => (d === "" ? null : d))}
        onDrop={handleRootDrop}
      >
        {rootState?.loading && entries.length === 0 ? (
          <FileTreeLoadingSkeleton label={t("common.loading", "Loading…")} depth={0} />
        ) : null}

        {draft?.dir === "" ? <DraftRow ctx={ctx} depth={0} /> : null}

        {entries.map((entry) => (
          <TreeNode key={entry.relPath} entry={entry} depth={0} ctx={ctx} />
        ))}

        {!rootState?.loading && entries.length === 0 && !draft ? (
          <div className="px-3 py-2 text-xs text-muted-foreground">
            {t("fileBrowser.empty", "This folder is empty")}
          </div>
        ) : null}
      </div>

      <AlertDialog open={!!deleteTarget} onOpenChange={(o) => !o && setDeleteTarget(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {deleteTarget?.isDir
                ? t("fileBrowser.confirmDeleteFolder", "Delete this folder and all its contents?")
                : t("fileBrowser.confirmDeleteFile", "Delete this file?")}
            </AlertDialogTitle>
            <AlertDialogDescription>{deleteTarget?.name}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel", "Cancel")}</AlertDialogCancel>
            <AlertDialogAction
              onClick={confirmDelete}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            >
              {t("common.delete", "Delete")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}

function TreeNode({
  entry,
  depth,
  ctx,
}: {
  entry: WorkspaceEntry
  depth: number
  ctx: TreeContext
}) {
  const { t } = useTranslation()
  const { fs, expansion } = ctx
  const expanded = entry.isDir && expansion.isExpanded(entry.relPath)
  const childState = expanded ? fs.getDir(entry.relPath) : undefined
  const rowRef = useRef<HTMLDivElement>(null)
  const fileTarget = useMemo<PreviewTarget>(
    () => ({
      kind: "workspace",
      scope: fs.scope.scope,
      scopeId: fs.scope.scopeId,
      relPath: entry.relPath,
      name: entry.name,
      sizeBytes: entry.size,
      isDirectory: entry.isDir,
    }),
    [entry, fs.scope.scope, fs.scope.scopeId],
  )
  const fileActions = useFileResource(fileTarget, {
    onPreviewFile: () => ctx.onSelectFile(entry),
    onEditFile: () => ctx.onEditFile?.(entry),
    onGuidedAction: () => ctx.onGuidedWrite?.(),
    workspaceAccess: fs.access ?? undefined,
    workspaceOperations: fs,
  })

  useEffect(() => {
    if (expanded && !childState) void fs.loadDir(entry.relPath)
  }, [expanded, childState, entry.relPath, fs])

  const icon = iconForEntry(entry.name, entry.isDir, expanded)
  const selected = ctx.selectedPath === entry.relPath
  const isRenaming = ctx.renaming === entry.relPath

  // Scroll the selected row into view (e.g. after a reveal expands ancestors to
  // it). `nearest` is a no-op when the row is already visible (ordinary clicks).
  useEffect(() => {
    if (selected) rowRef.current?.scrollIntoView({ block: "nearest" })
  }, [selected])

  const onActivate = useCallback(() => {
    if (entry.isDir) expansion.toggle(entry.relPath)
    else void fileActions.run(fileActions.primary)
  }, [entry.isDir, entry.relPath, expansion, fileActions])

  const commitRename = useCallback(
    async (nextName: string) => {
      ctx.setRenaming(null)
      const trimmed = nextName.trim()
      if (!trimmed || trimmed === entry.name) return
      const result = await fileActions.run("rename", {
        toPath: joinRel(parentDir(entry.relPath), trimmed),
      })
      if (result === "failed") toast.error(t("fileBrowser.renameFailed", "Rename failed"))
    },
    [ctx, entry, fileActions, t],
  )

  const onDrop = useCallback(
    async (e: DragEvent) => {
      if (!ctx.editable || !entry.isDir) return
      e.preventDefault()
      e.stopPropagation()
      ctx.setDragOverDir(null)
      const files = Array.from(e.dataTransfer.files)
      if (files.length === 0) return
      const result = await fileActions.run("upload", { dirPath: entry.relPath, files })
      if (result === "failed") toast.error(t("fileBrowser.uploadFailed", "Upload failed"))
    },
    [ctx, entry, fileActions, t],
  )

  const childEntries = childState?.entries ?? []

  return (
    <div>
      <ContextMenu>
        <ContextMenuTrigger asChild>
          <div
            ref={rowRef}
            className={cn(
              ROW,
              selected ? "bg-accent text-accent-foreground" : "hover:bg-accent/50",
              ctx.editable &&
                entry.isDir &&
                ctx.dragOverDir === entry.relPath &&
                "bg-accent/60 ring-1 ring-primary/40",
            )}
            style={{ paddingLeft: depth * INDENT + 6 }}
            onClick={onActivate}
            onDragOver={(e) => {
              if (!ctx.editable || !entry.isDir) return
              e.preventDefault()
              ctx.setDragOverDir(entry.relPath)
            }}
            onDragLeave={() => ctx.setDragOverDir(null)}
            onDrop={onDrop}
          >
            {entry.isDir ? (
              <ChevronRight
                className={cn(
                  "h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform duration-200 motion-reduce:transition-none",
                  expanded && "rotate-90",
                )}
              />
            ) : (
              <span className="w-3.5 shrink-0" />
            )}
            {entry.isDir ? (
              createElement(icon, { className: "h-3.5 w-3.5 shrink-0 text-muted-foreground" })
            ) : (
              <FileTypeIcon name={entry.name} className="h-3.5 w-3.5 shrink-0" />
            )}
            {isRenaming ? (
              <RenameInput
                initial={entry.name}
                onCommit={commitRename}
                onCancel={() => ctx.setRenaming(null)}
              />
            ) : (
              <span className="truncate">{entry.name}</span>
            )}
          </div>
        </ContextMenuTrigger>
        <ContextMenuContent variant="floating" className="w-48">
          {entry.isDir && ctx.editable ? (
            <>
              <ContextMenuItem
                onSelect={() => {
                  void fileActions.run("createFile", { prepareOnly: true }).then((result) => {
                    if (result !== "executed") return
                    ctx.expansion.setOpen(entry.relPath, true)
                    ctx.setDraft({ dir: entry.relPath, isDir: false })
                  })
                }}
              >
                {t("fileBrowser.newFile", "New File")}
              </ContextMenuItem>
              <ContextMenuItem
                onSelect={() => {
                  void fileActions.run("createFolder", { prepareOnly: true }).then((result) => {
                    if (result !== "executed") return
                    ctx.expansion.setOpen(entry.relPath, true)
                    ctx.setDraft({ dir: entry.relPath, isDir: true })
                  })
                }}
              >
                {t("fileBrowser.newFolder", "New Folder")}
              </ContextMenuItem>
              <ContextMenuSeparator />
            </>
          ) : null}
          {!entry.isDir ? (
            <>
              {fileActions.menu.map((action) => {
                const meta = FILE_ACTION_META[action]
                const Icon = meta.icon
                return (
                  <ContextMenuItem key={action} onSelect={() => fileActions.run(action)}>
                    <Icon className="mr-2 h-3.5 w-3.5" />
                    {t(meta.labelKey, meta.defaultLabel)}
                  </ContextMenuItem>
                )
              })}
              <ContextMenuSeparator />
            </>
          ) : null}
          {ctx.editable ? (
            <>
              <ContextMenuItem
                onSelect={() => {
                  void fileActions.run("rename", { prepareOnly: true }).then((result) => {
                    if (result === "executed") ctx.setRenaming(entry.relPath)
                  })
                }}
              >
                {t("fileBrowser.rename", "Rename")}
              </ContextMenuItem>
              <ContextMenuItem
                className="text-destructive focus:text-destructive"
                onSelect={() => {
                  void fileActions.run("delete", { prepareOnly: true }).then((result) => {
                    if (result === "executed") ctx.requestDelete(entry)
                  })
                }}
              >
                {t("fileBrowser.delete", "Delete")}
              </ContextMenuItem>
            </>
          ) : null}
        </ContextMenuContent>
      </ContextMenu>

      <AnimatedCollapse open={expanded} durationMs={160}>
        <div className="animate-in fade-in-0 slide-in-from-top-1 duration-150 motion-reduce:animate-none">
          {childState?.loading && childEntries.length === 0 ? (
            <FileTreeLoadingSkeleton depth={depth + 1} compact />
          ) : null}
          {ctx.draft?.dir === entry.relPath ? <DraftRow ctx={ctx} depth={depth + 1} /> : null}
          {childEntries.map((child) => (
            <TreeNode key={child.relPath} entry={child} depth={depth + 1} ctx={ctx} />
          ))}
        </div>
      </AnimatedCollapse>
    </div>
  )
}

function FileTreeLoadingSkeleton({
  depth,
  compact = false,
  label,
}: {
  depth: number
  compact?: boolean
  label?: string
}) {
  return (
    <div
      className={cn(
        "flex items-center gap-2 rounded-md py-1 text-xs text-muted-foreground",
        compact ? "h-7" : "h-8",
      )}
      style={{ paddingLeft: depth * INDENT + 6 }}
    >
      <Loader2 className="h-3 w-3 shrink-0 animate-spin motion-reduce:animate-none" />
      <div className="h-2.5 w-24 rounded-full bg-muted/70" />
      {label ? <span className="sr-only">{label}</span> : null}
    </div>
  )
}

/** Inline "new file / folder" input row. */
function DraftRow({ ctx, depth }: { ctx: TreeContext; depth: number }) {
  const { t } = useTranslation()
  const { fs, draft } = ctx
  const directoryTarget = useMemo<PreviewTarget>(
    () => ({
      kind: "workspace",
      scope: fs.scope.scope,
      scopeId: fs.scope.scopeId,
      relPath: draft?.dir ?? "",
      name: draft?.dir.split("/").pop() ?? "",
      isDirectory: true,
    }),
    [draft?.dir, fs.scope.scope, fs.scope.scopeId],
  )
  const directoryActions = useFileResource(directoryTarget, {
    workspaceAccess: fs.access ?? undefined,
    workspaceOperations: fs,
    onGuidedAction: () => ctx.onGuidedWrite?.(),
  })
  if (!draft) return null

  const onCommit = async (name: string) => {
    const trimmed = name.trim()
    ctx.setDraft(null)
    if (!trimmed) return
    const result = await directoryActions.run(draft.isDir ? "createFolder" : "createFile", {
      dirPath: draft.dir,
      name: trimmed,
    })
    if (result === "failed") toast.error(t("fileBrowser.createFailed", "Create failed"))
    else if (result === "executed" && !draft.isDir) {
      const relPath = joinRel(draft.dir, trimmed)
      ctx.onCreated?.({
        name: trimmed,
        relPath,
        isDir: false,
        isSymlink: false,
        size: 0,
        modifiedMs: Date.now(),
      })
    }
  }

  return (
    <div className={ROW} style={{ paddingLeft: depth * INDENT + 6 }}>
      <span className="w-3.5 shrink-0" />
      <span className="w-3.5 shrink-0" />
      <RenameInput
        initial=""
        placeholder={
          draft.isDir
            ? t("fileBrowser.newFolder", "New Folder")
            : t("fileBrowser.newFile", "New File")
        }
        onCommit={onCommit}
        onCancel={() => ctx.setDraft(null)}
      />
    </div>
  )
}

function RenameInput({
  initial,
  placeholder,
  onCommit,
  onCancel,
}: {
  initial: string
  placeholder?: string
  onCommit: (name: string) => void
  onCancel: () => void
}) {
  const ref = useRef<HTMLInputElement>(null)
  const [value, setValue] = useState(initial)
  // Enter commits and then unmounts this input, and the unmount fires `onBlur`
  // — without this guard the second `onCommit` runs against stale state and
  // surfaces a spurious "failed" toast for an operation that already succeeded.
  const doneRef = useRef(false)
  useEffect(() => {
    ref.current?.focus()
    ref.current?.select()
  }, [])
  const commit = (name: string) => {
    if (doneRef.current) return
    doneRef.current = true
    onCommit(name)
  }
  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      e.preventDefault()
      commit(value)
    } else if (e.key === "Escape") {
      e.preventDefault()
      doneRef.current = true // suppress the blur that follows the unmount
      onCancel()
    }
    e.stopPropagation()
  }
  return (
    <Input
      ref={ref}
      value={value}
      placeholder={placeholder}
      onChange={(e) => setValue(e.target.value)}
      onKeyDown={onKeyDown}
      onClick={(e) => e.stopPropagation()}
      onBlur={() => commit(value)}
      className="h-6 px-1 py-0 text-sm"
    />
  )
}
