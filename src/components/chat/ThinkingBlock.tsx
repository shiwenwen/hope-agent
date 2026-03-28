import { useState, useEffect } from "react"
import { useTranslation } from "react-i18next"
import { invoke } from "@tauri-apps/api/core"
import { cn } from "@/lib/utils"
import { ChevronRight, BrainCircuit } from "lucide-react"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"

// Module-level cache to avoid repeated invoke calls
let cachedAutoExpand: boolean | null = null
let cachePromise: Promise<boolean> | null = null

function getAutoExpandThinking(): Promise<boolean> {
  if (cachedAutoExpand !== null) return Promise.resolve(cachedAutoExpand)
  if (cachePromise) return cachePromise
  cachePromise = invoke<{ autoExpandThinking?: boolean }>("get_user_config")
    .then((cfg) => {
      cachedAutoExpand = cfg.autoExpandThinking !== false
      return cachedAutoExpand
    })
    .catch(() => {
      cachedAutoExpand = true
      return true
    })
  return cachePromise
}

/** Allow settings panel to invalidate cache when config changes */
export function invalidateThinkingExpandCache() {
  cachedAutoExpand = null
  cachePromise = null
}

interface ThinkingBlockProps {
  content: string
  isStreaming?: boolean
}

export default function ThinkingBlock({ content, isStreaming }: ThinkingBlockProps) {
  const { t } = useTranslation()
  const [autoExpand, setAutoExpand] = useState(cachedAutoExpand ?? true)
  const [isOpen, setIsOpen] = useState(!!isStreaming)
  const [prevStreaming, setPrevStreaming] = useState(isStreaming)

  // Load auto-expand setting
  useEffect(() => {
    if (cachedAutoExpand === null) {
      getAutoExpandThinking().then((v) => {
        setAutoExpand(v)
        // If setting loaded as false and not streaming, ensure collapsed
        if (!v && !isStreaming) {
          setIsOpen(false)
        }
      })
    }
  }, []) // eslint-disable-line react-hooks/exhaustive-deps

  // Auto-expand while streaming (only when autoExpand is on), auto-collapse when done
  if (isStreaming !== prevStreaming) {
    setPrevStreaming(isStreaming)
    if (isStreaming && autoExpand) {
      setIsOpen(true)
    } else if (!isStreaming && content) {
      setIsOpen(false)
    }
  }

  if (!content) return null

  return (
    <div className="mb-3">
      <button
        onClick={() => setIsOpen(!isOpen)}
        className="flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground transition-colors py-1 group"
      >
        <ChevronRight
          className={cn("h-3.5 w-3.5 transition-transform duration-200", isOpen && "rotate-90")}
        />
        <BrainCircuit
          className={cn("h-3.5 w-3.5", isStreaming && "animate-pulse text-purple-400")}
        />
        <span>{t("thinking.label")}</span>
        {isStreaming && <span className="text-[10px] text-purple-400 animate-pulse">···</span>}
      </button>

      <div
        className={cn(
          "overflow-hidden transition-all duration-300 ease-in-out",
          isOpen ? "max-h-[2000px] opacity-100" : "max-h-0 opacity-0",
        )}
      >
        <div className="ml-1 pl-3 border-l-2 border-purple-400/30 text-xs text-muted-foreground/80 leading-relaxed">
          <MarkdownRenderer content={content} isStreaming={isStreaming} />
        </div>
      </div>
    </div>
  )
}
