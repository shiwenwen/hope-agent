import { useCallback } from "react"
import { useTranslation } from "react-i18next"
import { Copy, Check } from "lucide-react"
import { useState } from "react"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"

interface SystemPromptDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  content: string
}

export default function SystemPromptDialog({
  open,
  onOpenChange,
  content,
}: SystemPromptDialogProps) {
  const { t } = useTranslation()
  const [copied, setCopied] = useState(false)

  const handleCopy = useCallback(() => {
    navigator.clipboard.writeText(content)
    setCopied(true)
    setTimeout(() => setCopied(false), 2000)
  }, [content])

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-4xl max-h-[80vh] flex flex-col">
        <DialogHeader className="flex flex-row items-center justify-between gap-2 pr-8">
          <DialogTitle>{t("chat.systemPrompt")}</DialogTitle>
          <button
            className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground transition-colors px-2 py-1 rounded-md hover:bg-secondary"
            onClick={handleCopy}
          >
            {copied ? (
              <Check className="h-3.5 w-3.5 text-green-500" />
            ) : (
              <Copy className="h-3.5 w-3.5" />
            )}
            {copied ? t("chat.copied") : t("chat.copy")}
          </button>
        </DialogHeader>
        <div className="flex-1 overflow-y-auto min-h-0">
          <pre className="text-xs leading-relaxed text-foreground/90 whitespace-pre-wrap break-words font-mono bg-secondary/30 rounded-lg p-4">
            {content}
          </pre>
        </div>
      </DialogContent>
    </Dialog>
  )
}
