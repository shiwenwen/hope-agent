import React, { useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { FileText } from "lucide-react"
import { TooltipProvider, IconTip } from "@/components/ui/tooltip"
import { logger } from "@/lib/logger"

function basename(filePath: string): string {
  const parts = filePath.replace(/\\/g, "/").split("/")
  return parts[parts.length - 1] || filePath
}

function FileAttachments({ files }: { files: string[] }) {
  const { t } = useTranslation()

  const handleOpen = useCallback(async (path: string) => {
    try {
      await invoke("open_directory", { path })
    } catch (e) {
      logger.error("chat", "FileAttachments::open", "Failed to open file", e)
    }
  }, [])

  if (files.length === 0) return null

  return (
    <div className="mt-2 pt-2 border-t border-border/30">
      <div className="text-[10px] text-muted-foreground/60 mb-1">
        {t("chat.modifiedFiles")}
      </div>
      <TooltipProvider>
        <div className="flex flex-wrap gap-1.5">
          {files.map((file) => (
            <IconTip key={file} label={file}>
              <button
                onClick={() => handleOpen(file)}
                className="inline-flex items-center gap-1 px-2 py-0.5 rounded-md bg-muted/50 hover:bg-muted text-xs text-foreground/70 hover:text-foreground transition-colors max-w-[200px]"
              >
                <FileText className="h-3 w-3 shrink-0 text-muted-foreground" />
                <span className="truncate">{basename(file)}</span>
              </button>
            </IconTip>
          ))}
        </div>
      </TooltipProvider>
    </div>
  )
}

export default React.memo(FileAttachments)
