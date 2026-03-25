import { useTranslation } from "react-i18next"
import { Copy } from "lucide-react"

interface MessageContextMenuProps {
  contextMenu: { x: number; y: number; index: number }
  onCopy: (index: number) => void
  onClose: () => void
}

export default function MessageContextMenu({
  contextMenu,
  onCopy,
  onClose,
}: MessageContextMenuProps) {
  const { t } = useTranslation()

  return (
    <div
      className="fixed z-[100] min-w-[140px] rounded-lg border border-border bg-popover p-1 shadow-lg animate-in fade-in-0 zoom-in-95"
      style={{ top: contextMenu.y, left: contextMenu.x }}
      onMouseDown={(e) => e.stopPropagation()}
    >
      <button
        className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-sm text-foreground hover:bg-muted/80 transition-colors"
        onClick={() => {
          onCopy(contextMenu.index)
          onClose()
        }}
      >
        <Copy className="h-3.5 w-3.5" />
        {t("chat.copy")}
      </button>
    </div>
  )
}
