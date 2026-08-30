import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Copy } from "lucide-react"

import { SelectionActionMenu } from "@/components/common/SelectionActionMenu"
import { FLOATING_MENU_ITEM_CLASS, FloatingMenu } from "@/components/ui/floating-menu"
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
  onAskInSideChat?: (quote: PendingMessageQuote) => void
  onClose: () => void
}

export default function MessageContextMenu({
  contextMenu,
  onCopy,
  onAddToChat,
  onAskInSideChat,
  onClose,
}: MessageContextMenuProps) {
  const { t } = useTranslation()
  const [lastWholeMessageSnapshot, setLastWholeMessageSnapshot] =
    useState<MessageContextMenuState | null>(null)
  const selectionSnapshot = contextMenu?.selectedText ? contextMenu : null
  const liveWholeMessageSnapshot = contextMenu && !contextMenu.selectedText ? contextMenu : null
  const wholeMessageOpen = Boolean(liveWholeMessageSnapshot)
  const wholeMessageSnapshot = liveWholeMessageSnapshot ?? lastWholeMessageSnapshot
  if (liveWholeMessageSnapshot && liveWholeMessageSnapshot !== lastWholeMessageSnapshot) {
    setLastWholeMessageSnapshot(liveWholeMessageSnapshot)
  }

  const selectionQuoteRole = selectionSnapshot?.quoteRole
  const selectionOpen = Boolean(contextMenu?.selectedText)

  return (
    <>
      <SelectionActionMenu
        open={selectionOpen}
        position={contextMenu?.selectedText ? { x: contextMenu.x, y: contextMenu.y } : null}
        text={contextMenu?.selectedText ?? ""}
        onCopy={(text) => {
          if (selectionSnapshot) onCopy(selectionSnapshot.index, text)
        }}
        onQuote={
          selectionQuoteRole && onAddToChat
            ? (text) => {
                onAddToChat({ role: selectionQuoteRole, content: text })
              }
            : undefined
        }
        onSideChat={
          selectionQuoteRole && onAskInSideChat
            ? (text) => {
                onAskInSideChat({ role: selectionQuoteRole, content: text })
              }
            : undefined
        }
        copyLabel={t("chat.copy")}
        quoteLabel={t("chat.messageQuote.addToChat", "添加到对话")}
        sideChatLabel={t("chat.sideChat.askSelection", "在侧聊中提问")}
        onClose={onClose}
        className="z-[100]"
      />

      <FloatingMenu
        open={wholeMessageOpen}
        strategy="fixed"
        portal
        onEscapeKeyDown={onClose}
        positionClassName=""
        originClassName="origin-top-left"
        className="z-[100] min-w-[140px] p-1.5"
        style={{
          top: wholeMessageSnapshot?.y ?? 0,
          left: wholeMessageSnapshot?.x ?? 0,
        }}
      >
        <div onPointerDown={(event) => event.stopPropagation()}>
          <button
            type="button"
            className={FLOATING_MENU_ITEM_CLASS}
            onClick={() => {
              if (wholeMessageSnapshot) onCopy(wholeMessageSnapshot.index)
              onClose()
            }}
          >
            <Copy className="h-3.5 w-3.5" aria-hidden="true" />
            {t("chat.copy")}
          </button>
        </div>
      </FloatingMenu>
    </>
  )
}
