/**
 * Project files panel: lists shared files, supports drag-drop upload and
 * per-file delete. Kept intentionally simple — one scrollable list with
 * inline actions. No rename UI in the first iteration (rename is supported
 * by the backend and `useProjectFiles` but not exposed here yet).
 */

import { useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { FileText, FileCode, FileImage, File as FileIcon, Trash2, Upload } from "lucide-react"

import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { useProjectFiles } from "./hooks/useProjectFiles"
import type { ProjectFile } from "@/types/project"

interface ProjectFilesPanelProps {
  projectId: string
}

export default function ProjectFilesPanel({ projectId }: ProjectFilesPanelProps) {
  const { t } = useTranslation()
  const { files, loading, error, uploadFile, deleteFile } = useProjectFiles(
    projectId,
  )
  const [dragOver, setDragOver] = useState(false)
  const [uploading, setUploading] = useState(false)
  const inputRef = useRef<HTMLInputElement>(null)

  async function handleFiles(list: FileList | File[]) {
    setUploading(true)
    try {
      for (const file of Array.from(list)) {
        await uploadFile(file)
      }
    } finally {
      setUploading(false)
    }
  }

  return (
    <div className="flex flex-col h-full gap-3">
      {/* Upload zone */}
      <div
        onDragOver={(e) => {
          e.preventDefault()
          setDragOver(true)
        }}
        onDragLeave={() => setDragOver(false)}
        onDrop={(e) => {
          e.preventDefault()
          setDragOver(false)
          if (e.dataTransfer.files.length > 0) {
            void handleFiles(e.dataTransfer.files)
          }
        }}
        onClick={() => inputRef.current?.click()}
        className={`border-2 border-dashed rounded-lg px-4 py-6 text-center cursor-pointer transition-colors ${
          dragOver
            ? "border-primary bg-primary/5"
            : "border-muted-foreground/25 hover:border-muted-foreground/50"
        }`}
      >
        <Upload className="mx-auto h-6 w-6 text-muted-foreground mb-2" />
        <p className="text-sm text-muted-foreground">
          {uploading ? t("project.files.uploading") : t("project.files.uploadHint")}
        </p>
        <input
          ref={inputRef}
          type="file"
          multiple
          hidden
          onChange={(e) => {
            if (e.target.files && e.target.files.length > 0) {
              void handleFiles(e.target.files)
              e.target.value = ""
            }
          }}
        />
      </div>

      {error && (
        <p className="text-sm text-destructive px-1">{error}</p>
      )}

      {/* File list */}
      <div className="flex-1 overflow-y-auto space-y-1 -mx-1 px-1">
        {loading && files.length === 0 ? (
          <p className="text-sm text-muted-foreground text-center py-4">...</p>
        ) : files.length === 0 ? (
          <p className="text-sm text-muted-foreground text-center py-8">
            {t("project.files.noFiles")}
          </p>
        ) : (
          files.map((file) => (
            <FileRow
              key={file.id}
              file={file}
              onDelete={() => {
                if (confirm(t("project.files.confirmDelete"))) {
                  void deleteFile(file.id)
                }
              }}
            />
          ))
        )}
      </div>
    </div>
  )
}

function FileRow({
  file,
  onDelete,
}: {
  file: ProjectFile
  onDelete: () => void
}) {
  const { t } = useTranslation()
  const Icon = iconForMime(file.mimeType)
  const sizeKb = (file.sizeBytes / 1024).toFixed(1)
  const extractedLabel =
    file.extractedChars && file.extractedChars > 0
      ? t("project.files.extractedPreview", { chars: file.extractedChars })
      : t("project.files.notExtractable")

  return (
    <div className="group flex items-center gap-2 px-2 py-2 rounded-md hover:bg-accent/40 transition-colors">
      <Icon className="h-4 w-4 text-muted-foreground shrink-0" />
      <div className="flex-1 min-w-0">
        <div className="text-sm font-medium truncate">{file.name}</div>
        <div className="text-xs text-muted-foreground truncate">
          {t("project.files.sizeKb", { size: sizeKb })} · {extractedLabel}
        </div>
      </div>
      <IconTip label={t("project.files.delete")}>
        <Button
          variant="ghost"
          size="sm"
          onClick={onDelete}
          className="opacity-0 group-hover:opacity-100 text-muted-foreground hover:text-destructive h-7 w-7 p-0"
        >
          <Trash2 className="h-3.5 w-3.5" />
        </Button>
      </IconTip>
    </div>
  )
}

function iconForMime(mime: string | null | undefined) {
  if (!mime) return FileIcon
  if (mime.startsWith("image/")) return FileImage
  if (
    mime.startsWith("text/") ||
    mime.includes("json") ||
    mime.includes("xml") ||
    mime.includes("javascript") ||
    mime.includes("typescript")
  )
    return FileCode
  if (mime === "application/pdf" || mime.includes("word") || mime.includes("sheet"))
    return FileText
  return FileIcon
}
