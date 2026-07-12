import { useState, useCallback, useEffect, useRef } from "react"
import { Send, MessageSquareQuote } from "lucide-react"
import { Button } from "@/components/ui/button"
import { FloatingMenu } from "@/components/ui/floating-menu"
import { Textarea } from "@/components/ui/textarea"
import { useTranslation } from "react-i18next"

/** Floating comment popover shown when user selects text in the plan */
export function CommentPopover({
  open,
  position,
  selectedText,
  onSubmit,
  onClose,
}: {
  open: boolean
  position: { top: number; left: number } | null
  selectedText: string | null
  onSubmit: (comment: string) => void
  onClose: () => void
}) {
  const { t } = useTranslation()
  const [comment, setComment] = useState("")
  const textareaRef = useRef<HTMLTextAreaElement>(null)
  const anchor = position && selectedText !== null ? { position, selectedText } : null

  useEffect(() => {
    if (!open) return
    const resetTimer = window.setTimeout(() => setComment(""), 0)
    const focusTimer = window.setTimeout(() => textareaRef.current?.focus(), 50)
    return () => {
      window.clearTimeout(resetTimer)
      window.clearTimeout(focusTimer)
    }
  }, [open])

  const handleSubmit = useCallback(() => {
    if (!comment.trim()) return
    onSubmit(comment.trim())
    setComment("")
  }, [comment, onSubmit])

  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault()
      handleSubmit()
    }
    if (e.key === "Escape") {
      e.preventDefault()
      onClose()
    }
  }, [handleSubmit, onClose])

  return (
    <FloatingMenu
      open={open && anchor !== null}
      positionClassName=""
      originClassName="origin-top-left"
      className="w-[280px] overflow-hidden p-0"
      style={{ top: anchor?.position.top ?? 0, left: anchor?.position.left ?? 0 }}
    >
      <div onMouseDown={(e) => e.stopPropagation()}>
      <div className="px-3 py-2 border-b border-border/50 bg-secondary/30 rounded-t-lg">
        <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <MessageSquareQuote className="h-3 w-3 shrink-0" />
          <span className="truncate italic">&ldquo;{(anchor?.selectedText.length ?? 0) > 60 ? anchor?.selectedText.slice(0, 60) + "…" : anchor?.selectedText}&rdquo;</span>
        </div>
      </div>
      <div className="p-2 space-y-2">
        <Textarea
          ref={textareaRef}
          value={comment}
          onChange={(e) => setComment(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={t("planMode.comment.placeholder")}
          className="text-sm min-h-[48px] max-h-[120px] resize-none border-border/50"
          rows={2}
        />
        <div className="flex items-center justify-between">
          <span className="text-[10px] text-muted-foreground">
            {t("planMode.comment.shortcut")}
          </span>
          <div className="flex gap-1.5">
            <Button size="sm" variant="ghost" className="h-7 px-2 text-xs" onClick={onClose}>
              {t("common.cancel")}
            </Button>
            <Button
              size="sm"
              className="h-7 px-2.5 text-xs gap-1"
              disabled={!comment.trim()}
              onClick={handleSubmit}
            >
              <Send className="h-3 w-3" />
              {t("planMode.comment.submit")}
            </Button>
          </div>
        </div>
      </div>
      </div>
    </FloatingMenu>
  )
}
