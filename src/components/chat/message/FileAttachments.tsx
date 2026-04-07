import React, { useCallback } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { FileText, FolderOpen } from "lucide-react"
import { IconTip } from "@/components/ui/tooltip"
import { logger } from "@/lib/logger"

function basename(filePath: string): string {
  const parts = filePath.replace(/\\/g, "/").split("/")
  return parts[parts.length - 1] || filePath
}

function FileAttachments({ files }: { files: string[] }) {
  const { t } = useTranslation()

  const handleOpen = useCallback(async (path: string) => {
    try {
      await getTransport().call("open_directory", { path })
    } catch (e) {
      logger.error("chat", "FileAttachments::open", "Failed to open file", e)
    }
  }, [])

  const handleRevealInFolder = useCallback(async (path: string) => {
    try {
      await getTransport().call("reveal_in_folder", { path })
    } catch (e) {
      logger.error("chat", "FileAttachments::reveal", "Failed to reveal in folder", e)
    }
  }, [])

  if (files.length === 0) return null

  return (
    <div className="mt-2 pt-2 border-t border-border/30">
      <div className="text-[10px] text-muted-foreground/60 mb-1">
        {t("chat.modifiedFiles")}
      </div>
      <div className="flex flex-wrap gap-1.5">
          {files.map((file) => (
            <span key={file} className="inline-flex items-center gap-0.5">
              <IconTip label={t("chat.openFile")}>
                <button
                  onClick={() => handleOpen(file)}
                  className="inline-flex items-center gap-1 pl-2 pr-1.5 py-0.5 rounded-l-md bg-muted/50 hover:bg-muted text-xs text-foreground/70 hover:text-foreground transition-colors max-w-[200px]"
                >
                  <FileText className="h-3 w-3 shrink-0 text-muted-foreground" />
                  <span className="truncate">{basename(file)}</span>
                </button>
              </IconTip>
              <IconTip label={t("chat.revealInFolder")}>
                <button
                  onClick={() => handleRevealInFolder(file)}
                  className="inline-flex items-center px-1 py-0.5 rounded-r-md bg-muted/50 hover:bg-muted text-foreground/70 hover:text-foreground transition-colors"
                >
                  <FolderOpen className="h-3 w-3 text-muted-foreground" />
                </button>
              </IconTip>
            </span>
          ))}
        </div>
    </div>
  )
}

export default React.memo(FileAttachments)
