import { useState, useEffect } from "react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { ChevronRight, BrainCircuit } from "lucide-react"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"

interface ThinkingBlockProps {
  content: string
  isStreaming?: boolean
}

export default function ThinkingBlock({ content, isStreaming }: ThinkingBlockProps) {
  const { t } = useTranslation()
  const [isOpen, setIsOpen] = useState(false)

  // Auto-expand while streaming, auto-collapse when done
  useEffect(() => {
    if (isStreaming) {
      setIsOpen(true)
    }
  }, [isStreaming])

  // Auto-collapse when streaming ends
  useEffect(() => {
    if (!isStreaming && content) {
      setIsOpen(false)
    }
  }, [isStreaming]) // eslint-disable-line react-hooks/exhaustive-deps

  if (!content) return null

  return (
    <div className="mb-3">
      <button
        onClick={() => setIsOpen(!isOpen)}
        className="flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground transition-colors py-1 group"
      >
        <ChevronRight
          className={cn(
            "h-3.5 w-3.5 transition-transform duration-200",
            isOpen && "rotate-90"
          )}
        />
        <BrainCircuit className={cn(
          "h-3.5 w-3.5",
          isStreaming && "animate-pulse text-purple-400"
        )} />
        <span>{t("thinking.label")}</span>
        {isStreaming && (
          <span className="text-[10px] text-purple-400 animate-pulse">
            ···
          </span>
        )}
      </button>

      <div
        className={cn(
          "overflow-hidden transition-all duration-300 ease-in-out",
          isOpen ? "max-h-[2000px] opacity-100" : "max-h-0 opacity-0"
        )}
      >
        <div className="ml-1 pl-3 border-l-2 border-purple-400/30 text-xs text-muted-foreground/80 leading-relaxed">
          <MarkdownRenderer content={content} isStreaming={isStreaming} />
        </div>
      </div>
    </div>
  )
}
