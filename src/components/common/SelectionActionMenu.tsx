import { useState } from "react"
import { Copy, Quote } from "lucide-react"
import { useTranslation } from "react-i18next"

import {
  FLOATING_MENU_ITEM_CLASS,
  FloatingMenu,
} from "@/components/ui/floating-menu"
import { cn } from "@/lib/utils"

export interface SelectionActionMenuPosition {
  /** Viewport-relative left coordinate used with fixed positioning. */
  x: number
  /** Viewport-relative top coordinate used with fixed positioning. */
  y: number
}

export interface SelectionActionMenuProps {
  open: boolean
  position: SelectionActionMenuPosition | null
  text: string
  onCopy: (text: string) => void | Promise<void>
  onQuote?: (text: string) => void
  quoteDisabled?: boolean
  onClose: () => void
  /** Context-menu opens without a selection can still offer a whole-item copy. */
  copyMode?: "selection" | "all"
  copyLabel?: string
  quoteLabel?: string
  className?: string
}

interface MenuSnapshot {
  position: SelectionActionMenuPosition
  text: string
  copyMode: "selection" | "all"
  quoteDisabled: boolean
}

/**
 * A selection-anchored copy/quote toolbar shared by previews and conversation
 * text. The last open snapshot is deliberately retained while closing so
 * FloatingMenu can finish its exit animation without jumping to (0, 0) or
 * losing its labels.
 *
 * Pointer-down is cancelled inside the toolbar. That keeps the browser's live
 * DOM Selection intact until the click handler has captured the selected text,
 * and also prevents an outside-pointer listener from dismissing the toolbar
 * before its action runs.
 */
export function SelectionActionMenu({
  open,
  position,
  text,
  onCopy,
  onQuote,
  quoteDisabled = false,
  onClose,
  copyMode = "selection",
  copyLabel,
  quoteLabel,
  className,
}: SelectionActionMenuProps) {
  const { t } = useTranslation()
  const [lastSnapshot, setLastSnapshot] = useState<MenuSnapshot>({
    position: { x: 0, y: 0 },
    text: "",
    copyMode: "selection",
    quoteDisabled: false,
  })

  const liveSnapshot = open && position ? { position, text, copyMode, quoteDisabled } : null
  const snapshot = liveSnapshot ?? lastSnapshot
  if (
    liveSnapshot &&
    (lastSnapshot.position.x !== liveSnapshot.position.x ||
      lastSnapshot.position.y !== liveSnapshot.position.y ||
      lastSnapshot.text !== liveSnapshot.text ||
      lastSnapshot.copyMode !== liveSnapshot.copyMode ||
      lastSnapshot.quoteDisabled !== liveSnapshot.quoteDisabled)
  ) {
    setLastSnapshot(liveSnapshot)
  }
  const hasText = snapshot.text.trim().length > 0
  const resolvedCopyLabel =
    copyLabel ??
    (snapshot.copyMode === "all"
      ? t("fileBrowser.copyAll", "Copy all")
      : t("fileBrowser.copySelection", "Copy selection"))
  const resolvedQuoteLabel = quoteLabel ?? t("fileBrowser.quoteToChat", "Quote to chat")

  return (
    <FloatingMenu
      open={open}
      strategy="fixed"
      portal
      onEscapeKeyDown={onClose}
      positionClassName=""
      originClassName="origin-bottom"
      className={cn("flex min-w-max items-center gap-0.5 p-1", className)}
      style={{ left: snapshot.position.x, top: snapshot.position.y }}
    >
      <div
        role="toolbar"
        aria-label={
          onQuote ? `${resolvedCopyLabel} / ${resolvedQuoteLabel}` : resolvedCopyLabel
        }
        className="flex items-center gap-0.5"
        onPointerDown={(event) => {
          event.preventDefault()
          event.stopPropagation()
        }}
      >
        <button
          type="button"
          disabled={!hasText}
          className={cn(FLOATING_MENU_ITEM_CLASS, "w-auto gap-1.5 px-2 py-1")}
          onClick={() => {
            void onCopy(snapshot.text)
            onClose()
          }}
        >
          <Copy className="h-3.5 w-3.5" aria-hidden="true" />
          {resolvedCopyLabel}
        </button>
        {onQuote ? (
          <button
            type="button"
            disabled={!hasText || snapshot.quoteDisabled}
            className={cn(FLOATING_MENU_ITEM_CLASS, "w-auto gap-1.5 px-2 py-1")}
            onClick={() => {
              onQuote(snapshot.text)
              onClose()
            }}
          >
            <Quote className="h-3.5 w-3.5" aria-hidden="true" />
            {resolvedQuoteLabel}
          </button>
        ) : null}
      </div>
    </FloatingMenu>
  )
}
