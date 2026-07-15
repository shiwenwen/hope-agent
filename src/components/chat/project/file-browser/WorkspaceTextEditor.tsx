import { defaultKeymap, history, historyKeymap } from "@codemirror/commands"
import { languages as codeLanguages } from "@codemirror/language-data"
import {
  LanguageDescription,
  syntaxHighlighting,
  defaultHighlightStyle,
} from "@codemirror/language"
import { EditorState, StateEffect } from "@codemirror/state"
import { EditorView, keymap, lineNumbers } from "@codemirror/view"
import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { AlertTriangle, Code2, ExternalLink, Eye, Loader2, Save, X } from "lucide-react"
import { toast } from "sonner"

import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { IconTip } from "@/components/ui/tooltip"
import { useTransport } from "@/lib/transport-provider"
import type { FileTextContent, FileWriteOutcome, WorkspaceEntry } from "@/lib/transport"
import type { ProjectFsApi } from "../hooks/useProjectFs"
import { MEBIBYTE_BYTES, useFilesystemConfig } from "@/lib/filesystemConfig"
import {
  clearFileEditorDirty,
  registerFileEditorDiscard,
  setFileEditorDirty,
} from "@/components/chat/files/fileDirtyRegistry"
import { useFileResource } from "@/components/chat/files/useFileResource"
import type { PreviewTarget } from "@/components/chat/files/useFilePreview"
import { dominantLineEnding, editorText, serializeText } from "./workspaceTextFormat"

const editorTheme = EditorView.theme({
  "&": { height: "100%", backgroundColor: "transparent" },
  ".cm-scroller": { overflow: "auto", fontFamily: "var(--font-mono)", fontSize: "12px" },
  ".cm-content": { padding: "12px 0" },
  ".cm-gutters": { backgroundColor: "transparent", borderRight: "1px solid var(--border)" },
  "&.cm-focused": { outline: "none" },
})

function suggestedCopyPath(path: string): string {
  const slash = path.lastIndexOf("/")
  const dir = slash >= 0 ? path.slice(0, slash + 1) : ""
  const name = slash >= 0 ? path.slice(slash + 1) : path
  const dot = name.lastIndexOf(".")
  return dot > 0 ? `${dir}${name.slice(0, dot)} copy${name.slice(dot)}` : `${dir}${name} copy`
}

export function WorkspaceTextEditor({
  fs,
  entry,
  onDirtyChange,
  onClose,
  onSavedAs,
  onGuidedWrite,
}: {
  fs: ProjectFsApi
  entry: WorkspaceEntry
  onDirtyChange: (dirty: boolean) => void
  onClose: () => void
  onSavedAs: (entry: WorkspaceEntry) => void
  onGuidedWrite?: () => void
}) {
  const { t } = useTranslation()
  const transport = useTransport()
  const { config: filesystemConfig } = useFilesystemConfig()
  const hostRef = useRef<HTMLDivElement | null>(null)
  const viewRef = useRef<EditorView | null>(null)
  const dataRef = useRef<FileTextContent | null>(null)
  const baseRef = useRef("")
  const dirtyRef = useRef(false)
  const savingRef = useRef(false)
  const saveRef = useRef<() => void>(() => {})
  const [data, setData] = useState<FileTextContent | null>(null)
  const [loading, setLoading] = useState(true)
  const [dirty, setDirty] = useState(false)
  const [previewText, setPreviewText] = useState("")
  const [markdownPreview, setMarkdownPreview] = useState(false)
  const [externalChanged, setExternalChanged] = useState(false)
  const [conflict, setConflict] = useState<Extract<
    FileWriteOutcome,
    { status: "conflict" }
  > | null>(null)
  const [saveAsOpen, setSaveAsOpen] = useState(false)
  const [saveAsPath, setSaveAsPath] = useState(() => suggestedCopyPath(entry.relPath))
  const editorInstanceId = useId()
  const dirtyRegistryId = `${fs.scope.scope}:${fs.scope.scopeId}:${entry.relPath}:${editorInstanceId}`
  const isMarkdown = /\.(md|markdown|mdown|mkd)$/i.test(entry.name)
  const target = useMemo<PreviewTarget>(
    () => ({
      kind: "workspace",
      scope: fs.scope.scope,
      scopeId: fs.scope.scopeId,
      relPath: entry.relPath,
      name: entry.name,
      sizeBytes: entry.size,
    }),
    [entry.name, entry.relPath, entry.size, fs.scope.scope, fs.scope.scopeId],
  )
  const fileActions = useFileResource(target, {
    workspaceAccess: fs.access ?? undefined,
    workspaceOperations: fs,
    onGuidedAction: onGuidedWrite,
    // The editor is already open; this callback keeps the shared `edit`
    // capability available as the pre-save gate without reopening anything.
    onEditFile: () => undefined,
  })

  useEffect(() => setMarkdownPreview(false), [entry.relPath])

  useEffect(() => {
    setFileEditorDirty(dirtyRegistryId, dirty)
    return () => clearFileEditorDirty(dirtyRegistryId)
  }, [dirty, dirtyRegistryId])

  useEffect(
    () =>
      registerFileEditorDiscard(dirtyRegistryId, () => {
        const view = viewRef.current
        const base = baseRef.current
        if (view && view.state.doc.toString() !== base) {
          view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: base } })
        }
        dirtyRef.current = false
        setDirty(false)
        setPreviewText(base)
        onDirtyChange(false)
      }),
    [dirtyRegistryId, onDirtyChange],
  )

  const applyLoaded = useCallback(
    (next: FileTextContent) => {
      const normalized = editorText(next.content)
      dataRef.current = next
      baseRef.current = normalized
      setPreviewText(normalized)
      setData(next)
      setDirty(false)
      dirtyRef.current = false
      onDirtyChange(false)
      setExternalChanged(false)
      const view = viewRef.current
      if (view && view.state.doc.toString() !== normalized) {
        view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: normalized } })
      }
    },
    [onDirtyChange],
  )

  const reload = useCallback(async () => {
    setLoading(true)
    try {
      const next = await fs.readFile(entry.relPath)
      if (
        next.isBinary ||
        !next.isUtf8 ||
        next.truncated ||
        next.sizeBytes > filesystemConfig.maxTextEditMb * MEBIBYTE_BYTES
      ) {
        throw new Error(t("fileEditor.notEditable", "This file cannot be edited as UTF-8 text"))
      }
      applyLoaded(next)
    } catch (error) {
      toast.error(error instanceof Error ? error.message : String(error))
    } finally {
      setLoading(false)
    }
  }, [applyLoaded, entry.relPath, filesystemConfig.maxTextEditMb, fs, t])

  useEffect(() => {
    void reload()
  }, [reload])

  const save = useCallback(async () => {
    const current = dataRef.current
    const view = viewRef.current
    if (!current?.contentHash || !view || !dirtyRef.current || savingRef.current) return
    savingRef.current = true
    try {
      const gate = await fileActions.run("edit", { prepareOnly: true })
      if (gate !== "executed") return
      const editorValue = view.state.doc.toString()
      const outcome = await fs.writeText(
        entry.relPath,
        serializeText(editorValue, current),
        current.contentHash,
      )
      if (outcome.status === "conflict") {
        setConflict(outcome)
        setExternalChanged(true)
        return
      }
      const next = {
        ...current,
        content: editorValue,
        contentHash: outcome.contentHash,
        sizeBytes: outcome.sizeBytes,
        lineEnding:
          current.lineEnding === "mixed" ? dominantLineEnding(current.content) : current.lineEnding,
      }
      dataRef.current = next
      baseRef.current = editorValue
      setData(next)
      setDirty(false)
      dirtyRef.current = false
      onDirtyChange(false)
      setExternalChanged(false)
      toast.success(t("fileEditor.saved", "Saved"))
    } catch (error) {
      toast.error(error instanceof Error ? error.message : String(error))
    } finally {
      savingRef.current = false
    }
  }, [entry.relPath, fileActions, fs, onDirtyChange, t])
  saveRef.current = () => void save()

  useEffect(() => {
    if (!hostRef.current || loading || !data || viewRef.current) return
    const state = EditorState.create({
      doc: baseRef.current,
      extensions: [
        history(),
        lineNumbers(),
        syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
        keymap.of([
          { key: "Mod-s", preventDefault: true, run: () => (saveRef.current(), true) },
          ...defaultKeymap,
          ...historyKeymap,
        ]),
        EditorView.lineWrapping,
        editorTheme,
        EditorView.contentAttributes.of({ "data-focus-ring": "none" }),
        EditorView.updateListener.of((update) => {
          if (!update.docChanged) return
          const next = update.state.doc.toString()
          const nextDirty = next !== baseRef.current
          dirtyRef.current = nextDirty
          setPreviewText(next)
          setDirty(nextDirty)
          onDirtyChange(nextDirty)
        }),
      ],
    })
    const view = new EditorView({ state, parent: hostRef.current })
    viewRef.current = view
    const description = LanguageDescription.matchFilename(codeLanguages, entry.name)
    if (description) {
      void description.load().then((support) => {
        if (viewRef.current === view) {
          view.dispatch({ effects: StateEffect.appendConfig.of(support) })
        }
      })
    }
    return () => {
      view.destroy()
      viewRef.current = null
    }
  }, [data, entry.name, loading, onDirtyChange])

  useEffect(() => {
    const beforeUnload = (event: BeforeUnloadEvent) => {
      if (!dirtyRef.current) return
      event.preventDefault()
    }
    window.addEventListener("beforeunload", beforeUnload)
    return () => window.removeEventListener("beforeunload", beforeUnload)
  }, [])

  const handleLegacyFsChanged = useCallback(async () => {
    if (savingRef.current) return
    try {
      const latest = await fs.readFile(entry.relPath)
      const currentHash = dataRef.current?.contentHash
      if (currentHash && latest.contentHash === currentHash) return
    } catch {
      // A missing/unreadable current file is still an external change. The
      // normal reload path below reports the concrete error for a clean editor.
    }
    if (dirtyRef.current) setExternalChanged(true)
    else void reload()
  }, [entry.relPath, fs, reload])

  useEffect(
    () =>
      transport.listen("project:fs_changed", (payload: unknown) => {
        const changed = payload as {
          scope?: string
          scopeId?: string
          dir?: string
          path?: string
        } | null
        if (!changed || changed.scope !== fs.scope.scope || changed.scopeId !== fs.scope.scopeId)
          return
        if (changed.path != null) {
          const changedPath = changed.path.replace(/^\/+/, "")
          if (changedPath !== entry.relPath || savingRef.current) return
          if (dirtyRef.current) setExternalChanged(true)
          else void reload()
          return
        }
        const parent = entry.relPath.includes("/")
          ? entry.relPath.slice(0, entry.relPath.lastIndexOf("/"))
          : ""
        if ((changed.dir ?? "") !== parent) return
        void handleLegacyFsChanged()
      }),
    [entry.relPath, fs.scope.scope, fs.scope.scopeId, handleLegacyFsChanged, reload, transport],
  )

  const doSaveAs = useCallback(async () => {
    const current = dataRef.current
    const view = viewRef.current
    const path = saveAsPath.trim().replace(/^\/+/, "")
    if (!current || !view || !path) return
    try {
      const content = serializeText(view.state.doc.toString(), current)
      const result = await fileActions.run("saveAs", { path, content })
      if (result !== "executed") {
        if (result === "failed") {
          toast.error(t("fileEditor.saveAsExists", "A file already exists at that path"))
        }
        return
      }
      const sizeBytes = new TextEncoder().encode(content).byteLength
      setSaveAsOpen(false)
      onSavedAs({
        name: path.split("/").pop() || path,
        relPath: path,
        isDir: false,
        isSymlink: false,
        size: sizeBytes,
        modifiedMs: Date.now(),
      })
    } catch (error) {
      toast.error(error instanceof Error ? error.message : String(error))
    }
  }, [fileActions, onSavedAs, saveAsPath, t])

  return (
    <div className="flex h-full min-w-0 flex-col">
      <div className="flex items-center gap-1.5 border-b px-3 py-1.5">
        <div className="min-w-0">
          <div className="truncate text-sm font-medium">
            {entry.name}
            {dirty ? <span className="ml-1 text-amber-500">●</span> : null}
          </div>
          <div className="truncate font-mono text-[11px] text-muted-foreground">
            {entry.relPath}
          </div>
        </div>
        <div className="ml-auto flex items-center gap-0.5">
          {isMarkdown ? (
            <IconTip
              label={
                markdownPreview
                  ? t("fileEditor.showSource", "Show source")
                  : t("fileEditor.showPreview", "Show preview")
              }
            >
              <Button
                size="icon"
                variant="ghost"
                className="h-6 w-6"
                onClick={() => setMarkdownPreview((value) => !value)}
              >
                {markdownPreview ? (
                  <Code2 className="h-3.5 w-3.5" />
                ) : (
                  <Eye className="h-3.5 w-3.5" />
                )}
              </Button>
            </IconTip>
          ) : null}
          <IconTip label={t("common.save", "Save")}>
            <Button
              size="icon"
              variant="ghost"
              className="h-6 w-6"
              disabled={!dirty}
              onClick={() => void save()}
            >
              <Save className="h-3.5 w-3.5" />
            </Button>
          </IconTip>
          <IconTip label={t("fileActions.saveAs", "Save as")}>
            <Button
              size="icon"
              variant="ghost"
              className="h-6 w-6"
              onClick={() => setSaveAsOpen(true)}
            >
              <ExternalLink className="h-3.5 w-3.5" />
            </Button>
          </IconTip>
          <IconTip label={t("common.close", "Close")}>
            <Button size="icon" variant="ghost" className="h-6 w-6" onClick={onClose}>
              <X className="h-3.5 w-3.5" />
            </Button>
          </IconTip>
        </div>
      </div>
      {data?.lineEnding === "mixed" ? (
        <div className="flex items-center gap-2 border-b bg-amber-500/10 px-3 py-1.5 text-xs text-amber-700 dark:text-amber-300">
          <AlertTriangle className="h-3.5 w-3.5" />
          {t(
            "fileEditor.mixedLineEndings",
            "Mixed line endings will be normalized to the dominant style when saved.",
          )}
        </div>
      ) : null}
      {externalChanged ? (
        <div className="flex items-center justify-between gap-2 border-b bg-amber-500/10 px-3 py-1.5 text-xs">
          <span>{t("fileEditor.externalChanged", "The file changed outside this editor.")}</span>
          <Button size="sm" variant="outline" className="h-6" onClick={() => void reload()}>
            {t("fileEditor.reload", "Reload")}
          </Button>
        </div>
      ) : null}
      <div className="min-h-0 flex-1">
        {loading ? (
          <div className="flex h-full items-center justify-center">
            <Loader2 className="h-4 w-4 animate-spin" />
          </div>
        ) : (
          <>
            <div ref={hostRef} className={markdownPreview ? "hidden" : "h-full"} />
            {markdownPreview ? (
              <div className="h-full overflow-auto px-5 py-4">
                <MarkdownRenderer content={previewText} />
              </div>
            ) : null}
          </>
        )}
      </div>

      <Dialog open={conflict != null} onOpenChange={(open) => !open && setConflict(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("fileEditor.conflictTitle", "Save conflict")}</DialogTitle>
            <DialogDescription>
              {conflict?.reason === "deleted"
                ? t("fileEditor.conflictDeleted", "The file was deleted outside this editor.")
                : t("fileEditor.conflictChanged", "The file was changed outside this editor.")}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setConflict(null)}>
              {t("common.cancel", "Cancel")}
            </Button>
            <Button
              variant="outline"
              onClick={() => {
                setConflict(null)
                setSaveAsOpen(true)
              }}
            >
              {t("fileActions.saveAs", "Save as")}
            </Button>
            <Button
              onClick={() => {
                setConflict(null)
                void reload()
              }}
            >
              {t("fileEditor.reload", "Reload")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={saveAsOpen} onOpenChange={setSaveAsOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("fileActions.saveAs", "Save as")}</DialogTitle>
            <DialogDescription>
              {t("fileEditor.saveAsScope", "Save a new file inside the current workspace.")}
            </DialogDescription>
          </DialogHeader>
          <Input
            value={saveAsPath}
            onChange={(event) => setSaveAsPath(event.target.value)}
            autoFocus
          />
          <DialogFooter>
            <Button variant="ghost" onClick={() => setSaveAsOpen(false)}>
              {t("common.cancel", "Cancel")}
            </Button>
            <Button onClick={() => void doSaveAs()}>{t("common.save", "Save")}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
