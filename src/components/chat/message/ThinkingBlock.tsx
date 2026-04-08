import { useState, useEffect, useMemo, useRef } from "react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { ChevronRight, BrainCircuit } from "lucide-react"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import { getAutoExpandThinking, getCachedAutoExpandThinking } from "../thinkingCache"

interface ThinkingBlockProps {
  content: string
  isStreaming?: boolean
  /** Persisted duration from DB (ms), used to display elapsed time after restart */
  durationMs?: number
}

export default function ThinkingBlock({ content, isStreaming, durationMs }: ThinkingBlockProps) {
  const { t } = useTranslation()
  const [autoExpand, setAutoExpand] = useState(getCachedAutoExpandThinking() ?? true)
  const [manualOpen, setManualOpen] = useState<boolean | null>(null)
  const [elapsedMs, setElapsedMs] = useState(0)
  const contentRef = useRef<HTMLDivElement>(null)
  const startedAtRef = useRef<number | null>(null)
  const isOpen = manualOpen ?? (isStreaming ? autoExpand : false)

  const formatElapsed = useMemo(
    () => (ms: number) => {
      if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`
      const totalSeconds = Math.floor(ms / 1000)
      const minutes = Math.floor(totalSeconds / 60)
      const seconds = totalSeconds % 60
      return `${minutes}m ${seconds}s`
    },
    [],
  )

  // Load auto-expand setting
  useEffect(() => {
    if (getCachedAutoExpandThinking() === null) {
      getAutoExpandThinking().then((v) => {
        setAutoExpand(v)
        // If setting loaded as false and not streaming, ensure collapsed
      })
    }
  }, [])

  useEffect(() => {
    if (isStreaming && !startedAtRef.current) {
      startedAtRef.current = Date.now()
    }
  }, [isStreaming])

  // Realtime elapsed timer while streaming
  useEffect(() => {
    if (!isStreaming || !startedAtRef.current) return
    const update = () => {
      setElapsedMs(Date.now() - startedAtRef.current!)
    }
    update()
    const timer = window.setInterval(update, 100)
    return () => window.clearInterval(timer)
  }, [isStreaming])

  // Keep elapsed frozen after complete
  useEffect(() => {
    if (!isStreaming && startedAtRef.current) {
      setElapsedMs(Date.now() - startedAtRef.current)
    }
  }, [isStreaming])

  // Auto-scroll inside thinking area when content grows
  useEffect(() => {
    if (!isOpen) return
    const container = contentRef.current
    if (!container) return
    container.scrollTop = container.scrollHeight
  }, [content, isOpen])

  if (!content) return null

  return (
    <div className="mb-3">
      <button
        onClick={() => setManualOpen((prev) => !(prev ?? (isStreaming ? autoExpand : false)))}
        className="flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground transition-colors py-1 group"
      >
        <ChevronRight
          className={cn("h-3.5 w-3.5 transition-transform duration-200", isOpen && "rotate-90")}
        />
        <BrainCircuit
          className={cn("h-3.5 w-3.5", isStreaming && "animate-pulse text-purple-400")}
        />
        <span className={cn(isStreaming && "animate-text-shimmer")}>{t("thinking.label")}</span>
        {(isStreaming || elapsedMs > 0 || (durationMs != null && durationMs > 0)) && (
          <span className="text-[10px] text-muted-foreground/70">{t("thinking.elapsed", { time: formatElapsed(elapsedMs > 0 ? elapsedMs : (durationMs || 0)) })}</span>
        )}
        {isStreaming && <span className="text-[10px] text-purple-400 animate-pulse">···</span>}
      </button>

      <div
        className={cn(
          "overflow-hidden transition-all duration-300 ease-in-out",
          isOpen ? "max-h-[360px] opacity-100" : "max-h-0 opacity-0",
        )}
      >
        <div
          ref={contentRef}
          className="ml-1 pl-3 border-l-2 border-purple-400/30 text-xs text-muted-foreground/80 leading-relaxed max-h-[320px] overflow-y-auto pr-2"
        >
          <MarkdownRenderer content={content} isStreaming={isStreaming} />
        </div>
      </div>
    </div>
  )
}
