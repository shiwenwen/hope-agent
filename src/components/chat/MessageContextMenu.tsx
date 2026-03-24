import { useTranslation } from "react-i18next"
import { Copy, Pencil } from "lucide-react"

interface MessageContextMenuProps {
  contextMenu: { x: number; y: number; index: number }
  onStartEdit: (index: number) => void
  onCopy: (index: number) => void
  onClose: () => void
}

export default function MessageContextMenu({
  contextMenu,
  onStartEdit,
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
        onClick={() => onStartEdit(contextMenu.index)}
      >
        <Pencil className="h-3.5 w-3.5" />
        {t("common.edit")}
      </button>
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
