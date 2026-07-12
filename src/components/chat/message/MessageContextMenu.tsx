import { useTranslation } from "react-i18next"
import { Copy, Quote } from "lucide-react"
import { FloatingMenu } from "@/components/ui/floating-menu"
import type { PendingMessageQuote } from "@/types/chat"

export interface MessageContextMenuState {
  x: number
  y: number
  index: number
  selectedText?: string
  quoteRole?: PendingMessageQuote["role"]
}

interface MessageContextMenuProps {
  contextMenu: MessageContextMenuState | null
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

  return (
    <FloatingMenu
      open={contextMenu !== null}
      strategy="fixed"
      portal
      positionClassName=""
      originClassName="origin-top-left"
      className="z-[100] min-w-[140px] p-1.5"
      style={{ top: contextMenu?.y ?? 0, left: contextMenu?.x ?? 0 }}
    >
      <div onMouseDown={(event) => event.stopPropagation()}>
        <button
          className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-[13px] text-foreground/80 transition-colors hover:bg-secondary/60 hover:text-foreground"
          onClick={() => {
            if (contextMenu) onCopy(contextMenu.index, contextMenu.selectedText)
            onClose()
          }}
        >
          <Copy className="h-3.5 w-3.5" />
          {t("chat.copy")}
        </button>
        {contextMenu?.selectedText && contextMenu.quoteRole && onAddToChat ? (
          <button
            className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-[13px] text-foreground/80 transition-colors hover:bg-secondary/60 hover:text-foreground"
            onClick={() => {
              const quoteRole = contextMenu?.quoteRole
              const selectedText = contextMenu?.selectedText
              if (!quoteRole || !selectedText) return
              onAddToChat({
                role: quoteRole,
                content: selectedText,
              })
              onClose()
            }}
          >
            <Quote className="h-3.5 w-3.5" />
            {t("chat.messageQuote.addToChat", "添加到对话")}
          </button>
        ) : null}
      </div>
    </FloatingMenu>
  )
}
