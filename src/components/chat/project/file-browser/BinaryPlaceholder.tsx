/** Fallback preview for binary / oversized files: icon, size, and actions. */

import { createElement } from "react"
import { useTranslation } from "react-i18next"
import { Download, ExternalLink } from "lucide-react"

import { Button } from "@/components/ui/button"
import { formatBytes } from "@/lib/format"
import { iconForEntry } from "@/lib/fileKind"

export function BinaryPlaceholder({
  name,
  sizeBytes,
  note,
  onOpen,
  onDownload,
}: {
  name: string
  sizeBytes: number
  note?: string
  onOpen?: () => void
  onDownload?: () => void
}) {
  const { t } = useTranslation()
  const icon = iconForEntry(name, false)
  return (
    <div className="flex h-full flex-col items-center justify-center gap-3 px-6 text-center">
      {createElement(icon, { className: "h-12 w-12 text-muted-foreground" })}
      <div className="space-y-0.5">
        <div className="text-sm font-medium">{name}</div>
        <div className="text-xs text-muted-foreground">{formatBytes(sizeBytes, { fractionDigits: 1 })}</div>
        <div className="text-xs text-muted-foreground">
          {note ?? t("fileBrowser.binaryFile", "Binary file")}
        </div>
      </div>
      <div className="flex gap-2">
        {onOpen ? (
          <Button size="sm" variant="outline" onClick={onOpen}>
            <ExternalLink className="mr-1.5 h-3.5 w-3.5" />
            {t("fileBrowser.openInSystem", "Open in system")}
          </Button>
        ) : null}
        {onDownload ? (
          <Button size="sm" variant="outline" onClick={onDownload}>
            <Download className="mr-1.5 h-3.5 w-3.5" />
            {t("fileBrowser.download", "Download")}
          </Button>
        ) : null}
      </div>
    </div>
  )
}
