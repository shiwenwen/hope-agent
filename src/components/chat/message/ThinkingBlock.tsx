import { useState, useEffect, useRef } from "react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { ChevronRight, BrainCircuit } from "lucide-react"
import { AnimatedCollapse } from "@/components/ui/animated-presence"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import { getAutoExpandThinking, getCachedAutoExpandThinking } from "../thinkingCache"
import { formatDuration } from "../chatUtils"
import InterruptedMark from "./InterruptedMark"

interface ThinkingBlockProps {
  content: string
  compact?: boolean
  isStreaming?: boolean
  /** Persisted duration from DB (ms), used to display elapsed time after restart */
  durationMs?: number
  /** Set when this thinking block was left mid-stream by a crashed run; renders
   *  an "interrupted" mark below the content. */
  interrupted?: boolean
}

export default function ThinkingBlock({
  content,
  compact,
  isStreaming,
  durationMs,
  interrupted,
}: ThinkingBlockProps) {
  const { t } = useTranslation()
  const [autoExpand, setAutoExpand] = useState(getCachedAutoExpandThinking() ?? true)
  const [manualOpen, setManualOpen] = useState<boolean | null>(null)
  const [elapsedMs, setElapsedMs] = useState(0)
  const contentRef = useRef<HTMLDivElement>(null)
  const startedAtRef = useRef<number | null>(null)
  const isOpen = manualOpen ?? (isStreaming ? autoExpand : false)

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
    <div className={cn(compact ? "mb-1" : "mb-3")}>
      <button
        onClick={() => setManualOpen((prev) => !(prev ?? (isStreaming ? autoExpand : false)))}
        className={cn(
          "flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground transition-colors group",
          compact ? "py-0.5" : "py-1",
        )}
      >
        <ChevronRight
          className={cn("h-3.5 w-3.5 transition-transform duration-200", isOpen && "rotate-90")}
        />
        <BrainCircuit
          className={cn("h-3.5 w-3.5", isStreaming && "animate-pulse text-purple-400")}
        />
        <span className={cn(isStreaming && "animate-text-shimmer")}>
          {t(isStreaming ? "thinking.streaming" : "thinking.done")}
        </span>
        {(isStreaming || elapsedMs > 0 || (durationMs != null && durationMs > 0)) && (
          <span className="text-[10px] text-muted-foreground/70">
            {t("thinking.elapsed", {
              time: formatDuration(elapsedMs > 0 ? elapsedMs : durationMs || 0),
            })}
          </span>
        )}
        {isStreaming && <span className="text-[10px] text-purple-400 animate-pulse">···</span>}
      </button>

      <AnimatedCollapse open={isOpen} unmountOnExit={false}>
        <div
          ref={contentRef}
          className="ml-1 pl-3 border-l-2 border-purple-400/30 text-xs text-muted-foreground/80 leading-relaxed max-h-[320px] overflow-y-auto pr-2"
        >
          <MarkdownRenderer content={content} isStreaming={isStreaming} />
        </div>
      </AnimatedCollapse>
      {interrupted ? <InterruptedMark className="ml-6" /> : null}
    </div>
  )
}
