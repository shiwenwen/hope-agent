import { useCallback, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"

import { FilePreviewPane } from "@/components/chat/project/file-browser/FilePreviewPane"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Textarea } from "@/components/ui/textarea"
import { stagedFilePreviewSource } from "./previewSource"
import { useFileActionsContext } from "./fileActionsContext"
import { useFileResource } from "./useFileResource"
import { useObjectUrlLease } from "./useObjectUrlLease"
import type { PreviewTarget } from "./useFilePreview"
import { MEBIBYTE_BYTES, useFilesystemConfig } from "@/lib/filesystemConfig"

type StagedPreviewTarget = Extract<PreviewTarget, { kind: "clientDraft" }>

export function StagedFilePreviewPane({
  target,
  onClose,
  className,
  maximized,
  onToggleMaximize,
  onReplaceFile,
}: {
  target: StagedPreviewTarget
  onClose?: () => void
  className?: string
  maximized?: boolean
  onToggleMaximize?: () => void
  onReplaceFile?: (file: File) => void
}) {
  const { t } = useTranslation()
  const actionsContext = useFileActionsContext()
  const { config: filesystemConfig } = useFilesystemConfig()
  const [file, setFile] = useState(target.draft.file)
  const [trackedDraftFile, setTrackedDraftFile] = useState(target.draft.file)
  const [editOpen, setEditOpen] = useState(false)
  const [editText, setEditText] = useState("")
  const [hadBom, setHadBom] = useState(false)
  if (trackedDraftFile !== target.draft.file) {
    setTrackedDraftFile(target.draft.file)
    setFile(target.draft.file)
  }
  const objectUrl = useObjectUrlLease(file)

  const source = useMemo(
    () =>
      stagedFilePreviewSource(
        file,
        objectUrl ?? "",
        filesystemConfig.maxTextPreviewMb * MEBIBYTE_BYTES,
        filesystemConfig.maxDocumentPreviewMb * MEBIBYTE_BYTES,
      ),
    [file, objectUrl, filesystemConfig.maxDocumentPreviewMb, filesystemConfig.maxTextPreviewMb],
  )

  const openEditor = useCallback(async () => {
    try {
      const bytes = new Uint8Array(await file.arrayBuffer())
      const bom = bytes.length >= 3 && bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf
      const text = new TextDecoder("utf-8", { fatal: true }).decode(bom ? bytes.subarray(3) : bytes)
      setHadBom(bom)
      setEditText(text)
      setEditOpen(true)
    } catch {
      toast.error(t("fileEditor.notUtf8", "File is not valid UTF-8 text"))
    }
  }, [file, t])
  const activeTarget = useMemo<StagedPreviewTarget>(
    () => ({ ...target, draft: { ...target.draft, file } }),
    [file, target],
  )
  const fileActions = useFileResource(activeTarget, {
    onEditFile: () => void openEditor(),
  })

  const saveEdit = () => {
    const next = new File([hadBom ? `\uFEFF${editText}` : editText], file.name, {
      type: file.type || "text/plain",
      lastModified: Date.now(),
    })
    if (next.size > filesystemConfig.maxTextEditMb * MEBIBYTE_BYTES) {
      toast.error(
        t("fileEditor.tooLarge", "File exceeds the {{limit}} MiB edit limit", {
          limit: filesystemConfig.maxTextEditMb,
        }),
      )
      return
    }
    setFile(next)
    if (onReplaceFile) onReplaceFile(next)
    else actionsContext.onReplaceDraft?.(target.draft.id, next)
    setEditOpen(false)
    toast.success(t("common.saved", "Saved"))
  }

  return (
    <>
      <FilePreviewPane
        source={source}
        onClose={onClose}
        onOpen={() => void fileActions.run("open")}
        onDownload={() => void fileActions.run("download")}
        onEdit={
          fileActions.capabilities.edit.state === "enabled"
            ? () => void fileActions.run("edit")
            : undefined
        }
        className={className}
        maximized={maximized}
        onToggleMaximize={onToggleMaximize}
      />
      <Dialog open={editOpen} onOpenChange={setEditOpen}>
        <DialogContent className="flex h-[min(82vh,40rem)] max-w-4xl flex-col">
          <DialogHeader>
            <DialogTitle>{file.name}</DialogTitle>
          </DialogHeader>
          <Textarea
            value={editText}
            onChange={(event) => setEditText(event.target.value)}
            spellCheck={false}
            className="min-h-0 flex-1 resize-none font-mono text-xs"
          />
          <DialogFooter>
            <Button variant="ghost" onClick={() => setEditOpen(false)}>
              {t("common.cancel", "Cancel")}
            </Button>
            <Button onClick={saveEdit}>{t("common.save", "Save")}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  )
}
