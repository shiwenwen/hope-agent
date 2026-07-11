import { useTranslation } from "react-i18next"
import { Copy, Quote } from "lucide-react"
import { createPortal } from "react-dom"
import type { PendingMessageQuote } from "@/types/chat"

export interface MessageContextMenuState {
  x: number
  y: number
  index: number
  selectedText?: string
  quoteRole?: PendingMessageQuote["role"]
}

interface MessageContextMenuProps {
  contextMenu: MessageContextMenuState
  onCopy: (index: number, selectedText?: string) => void
  onAddToChat?: (quote: PendingMessageQuote) => void
  onClose: () => void
}

export default function MessageContextMenu({
  contextMenu,
  onCopy,
  onAddToChat,
  onClose,
}: MessageContextMenuProps) {
  const { t } = useTranslation()

  return createPortal(
    <div
      className="fixed z-[100] min-w-[140px] rounded-lg border border-border bg-popover p-1 shadow-lg animate-in fade-in-0 zoom-in-95"
      style={{ top: contextMenu.y, left: contextMenu.x }}
      onPointerDown={(e) => e.stopPropagation()}
    >
      <button
        type="button"
        className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-sm text-foreground hover:bg-muted/80 transition-colors"
        onClick={() => {
          onCopy(contextMenu.index, contextMenu.selectedText)
          onClose()
        }}
      >
        <Copy className="h-3.5 w-3.5" />
        {t("chat.copy")}
      </button>
      {contextMenu.selectedText && contextMenu.quoteRole && onAddToChat ? (
        <button
          type="button"
          className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-sm text-foreground hover:bg-muted/80 transition-colors"
          onClick={() => {
            onAddToChat({ role: contextMenu.quoteRole!, content: contextMenu.selectedText! })
            onClose()
          }}
        >
          <Quote className="h-3.5 w-3.5" />
          {t("chat.messageQuote.addToChat", "添加到对话")}
        </button>
      ) : null}
    </div>,
    document.body,
  )
}
