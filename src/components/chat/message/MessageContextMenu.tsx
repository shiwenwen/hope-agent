import { useRef } from "react"
import { useTranslation } from "react-i18next"
import { Copy } from "lucide-react"
import { FloatingMenu } from "@/components/ui/floating-menu"

interface MessageContextMenuProps {
  contextMenu: { x: number; y: number; index: number } | null
  onCopy: (index: number) => void
  onClose: () => void
}

export default function MessageContextMenu({
  contextMenu,
  onCopy,
  onClose,
}: MessageContextMenuProps) {
  const { t } = useTranslation()
  const lastMenuRef = useRef(contextMenu)
  if (contextMenu) lastMenuRef.current = contextMenu
  const renderedMenu = contextMenu ?? lastMenuRef.current

  if (!renderedMenu) return null

  return (
    <FloatingMenu
      open={contextMenu !== null}
      strategy="fixed"
      portal
      positionClassName=""
      originClassName="origin-top-left"
      className="z-[100] min-w-[140px] p-1.5"
      style={{ top: renderedMenu.y, left: renderedMenu.x }}
    >
      <div onMouseDown={(e) => e.stopPropagation()}>
        <button
          className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-[13px] text-foreground/80 transition-colors hover:bg-secondary/60 hover:text-foreground"
          onClick={() => {
            onCopy(renderedMenu.index)
            onClose()
          }}
        >
          <Copy className="h-3.5 w-3.5" />
          {t("chat.copy")}
        </button>
      </div>
    </FloatingMenu>
  )
}
