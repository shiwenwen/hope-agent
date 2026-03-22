import { useState, useEffect } from "react"
import { ChevronDown, ChevronRight, Users, CheckCircle, XCircle, Clock, Loader2, Skull } from "lucide-react"
import { listen, type UnlistenFn } from "@tauri-apps/api/event"
import type { SubagentEvent } from "@/types/chat"

interface SubagentBlockProps {
  runId: string
  agentId: string
  task: string
  initialStatus?: string
  onNavigateToSession?: (sessionId: string) => void
}

const statusConfig: Record<string, { icon: React.ReactNode; label: string; color: string }> = {
  spawning: { icon: <Loader2 className="h-3 w-3 animate-spin" />, label: "Spawning", color: "text-blue-500" },
  running: { icon: <Loader2 className="h-3 w-3 animate-spin" />, label: "Running", color: "text-blue-500" },
  completed: { icon: <CheckCircle className="h-3 w-3" />, label: "Completed", color: "text-green-500" },
  error: { icon: <XCircle className="h-3 w-3" />, label: "Error", color: "text-red-500" },
  timeout: { icon: <Clock className="h-3 w-3" />, label: "Timeout", color: "text-orange-500" },
  killed: { icon: <Skull className="h-3 w-3" />, label: "Killed", color: "text-gray-500" },
}

export default function SubagentBlock({ runId, agentId, task, initialStatus }: SubagentBlockProps) {
  const [expanded, setExpanded] = useState(false)
  const [status, setStatus] = useState(initialStatus || "spawning")
  const [resultPreview, setResultPreview] = useState<string | undefined>()
  const [error, setError] = useState<string | undefined>()
  const [durationMs, setDurationMs] = useState<number | undefined>()

  useEffect(() => {
    let unlisten: UnlistenFn | undefined
    listen<SubagentEvent>("subagent_event", (event) => {
      const payload = event.payload
      if (payload.runId !== runId) return
      setStatus(payload.status)
      if (payload.resultPreview) setResultPreview(payload.resultPreview)
      if (payload.error) setError(payload.error)
      if (payload.durationMs) setDurationMs(payload.durationMs)
    }).then((fn) => {
      unlisten = fn
    })
    return () => {
      unlisten?.()
    }
  }, [runId])

  const isTerminal = ["completed", "error", "timeout", "killed"].includes(status)
  const config = statusConfig[status] || statusConfig.error

  return (
    <div className="my-1.5 rounded-lg border border-border bg-secondary/50 text-xs">
      <button
        className="flex items-center gap-1.5 w-full px-2.5 py-1.5 text-left hover:bg-secondary/80 rounded-lg transition-colors"
        onClick={() => isTerminal && setExpanded(!expanded)}
      >
        {!isTerminal ? (
          <span className="animate-spin h-3 w-3 border border-current border-t-transparent rounded-full shrink-0" />
        ) : expanded ? (
          <ChevronDown className="h-3 w-3 shrink-0 text-muted-foreground" />
        ) : (
          <ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground" />
        )}
        <Users className="h-3 w-3 shrink-0 text-muted-foreground" />
        <span className="font-medium text-foreground">subagent</span>
        <span className="text-muted-foreground truncate flex-1">{agentId}: {task}</span>
        <span className={`flex items-center gap-1 ${config.color}`}>
          {config.icon}
          <span>{config.label}</span>
        </span>
        {durationMs !== undefined && (
          <span className="text-muted-foreground">{(durationMs / 1000).toFixed(1)}s</span>
        )}
      </button>
      {expanded && (resultPreview || error) && (
        <div className="px-2.5 pb-2 pt-0.5">
          {error && (
            <pre className="whitespace-pre-wrap text-red-400 bg-background rounded p-2 max-h-48 overflow-y-auto text-[11px] leading-relaxed">
              {error}
            </pre>
          )}
          {resultPreview && (
            <pre className="whitespace-pre-wrap text-muted-foreground bg-background rounded p-2 max-h-48 overflow-y-auto text-[11px] leading-relaxed">
              {resultPreview}
            </pre>
          )}
        </div>
      )}
    </div>
  )
}
