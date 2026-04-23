import { useState, useEffect } from "react"
import { ChevronRight, Cable, CheckCircle, XCircle, Clock, Loader2, Skull, StopCircle } from "lucide-react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { getTransport } from "@/lib/transport-provider"
import { Button } from "@/components/ui/button"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"

interface AcpSpawnBlockProps {
  runId: string
  backendId: string
  task: string
  initialStatus?: string
  label?: string
}

interface AcpControlEvent {
  eventType: string
  runId: string
  parentSessionId: string
  backendId: string
  label?: string
  data: Record<string, unknown>
}

const statusConfig: Record<string, { icon: React.ReactNode; color: string }> = {
  starting: {
    icon: <Loader2 className="h-3 w-3 animate-spin" />,
    color: "text-blue-500",
  },
  running: {
    icon: <Loader2 className="h-3 w-3 animate-spin" />,
    color: "text-blue-500",
  },
  completed: {
    icon: <CheckCircle className="h-3 w-3" />,
    color: "text-green-500",
  },
  error: { icon: <XCircle className="h-3 w-3" />, color: "text-red-500" },
  timeout: { icon: <Clock className="h-3 w-3" />, color: "text-orange-500" },
  killed: { icon: <Skull className="h-3 w-3" />, color: "text-gray-500" },
}

export default function AcpSpawnBlock({
  runId,
  backendId,
  task,
  initialStatus,
  label: initialLabel,
}: AcpSpawnBlockProps) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const [status, setStatus] = useState(initialStatus || "starting")
  const [streamText, setStreamText] = useState("")
  const [error, setError] = useState<string | undefined>()
  const [durationMs, setDurationMs] = useState<number | undefined>()
  const [label] = useState(initialLabel)
  const [inputTokens, setInputTokens] = useState<number | undefined>()
  const [outputTokens, setOutputTokens] = useState<number | undefined>()
  const [toolCalls, setToolCalls] = useState<string[]>([])

  const isTerminal = ["completed", "error", "timeout", "killed"].includes(status)
  const config = statusConfig[status] || statusConfig.starting

  // Listen for ACP control events
  useEffect(() => {
    return getTransport().listen("acp_control_event", (raw) => {
      const payload = raw as AcpControlEvent
      if (payload.runId !== runId) return

      switch (payload.eventType) {
        case "text_delta":
          setStatus("running")
          setStreamText((prev) => prev + ((payload.data.content as string) || ""))
          break
        case "tool_call":
          setToolCalls((prev) => [...prev, (payload.data.name as string) || "unknown"])
          break
        case "usage":
          setInputTokens(payload.data.inputTokens as number)
          setOutputTokens(payload.data.outputTokens as number)
          break
        case "completed":
          setStatus("completed")
          setDurationMs(payload.data.durationMs as number)
          setInputTokens(payload.data.inputTokens as number)
          setOutputTokens(payload.data.outputTokens as number)
          break
        case "error":
          setStatus("error")
          setError((payload.data.error as string) || (payload.data.message as string))
          setDurationMs(payload.data.durationMs as number)
          break
        case "timeout":
          setStatus("timeout")
          setDurationMs(payload.data.durationMs as number)
          break
        case "killed":
          setStatus("killed")
          break
      }
    })
  }, [runId])

  const handleKill = async () => {
    try {
      await getTransport().call("acp_kill_run", { runId })
    } catch {
      // ignore
    }
  }

  const formatDuration = (ms: number) => {
    if (ms < 1000) return `${ms}ms`
    return `${(ms / 1000).toFixed(1)}s`
  }

  const formatTokens = (input?: number, output?: number) => {
    if (!input && !output) return null
    return `${(input || 0).toLocaleString()} → ${(output || 0).toLocaleString()} tokens`
  }

  return (
    <div className="my-1 rounded-md border bg-card/50 text-xs">
      {/* Header */}
      <button
        className="flex w-full items-center gap-2 px-3 py-2 text-left hover:bg-muted/50 transition-colors"
        onClick={() => setExpanded(!expanded)}
      >
        <ChevronRight
          className={cn("h-3 w-3 shrink-0 transition-transform", expanded && "rotate-90")}
        />
        <Cable className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <span className="font-medium truncate">
          {label || backendId}
        </span>
        <span className="text-muted-foreground truncate max-w-[200px]">{task}</span>
        <div className="ml-auto flex items-center gap-2 shrink-0">
          {durationMs != null && (
            <span className="text-muted-foreground">{formatDuration(durationMs)}</span>
          )}
          {formatTokens(inputTokens, outputTokens) && (
            <span className="text-muted-foreground">{formatTokens(inputTokens, outputTokens)}</span>
          )}
          <span className={cn("flex items-center gap-1", config.color)}>
            {config.icon}
            {t(`executionStatus.acp.status.${status}`, status)}
          </span>
          {!isTerminal && (
            <Button
              variant="ghost"
              size="icon"
              className="h-5 w-5"
              onClick={(e) => {
                e.stopPropagation()
                handleKill()
              }}
            >
              <StopCircle className="h-3 w-3 text-red-500" />
            </Button>
          )}
        </div>
      </button>

      {/* Expanded content */}
      {expanded && (
        <div className="border-t px-3 py-2 space-y-2">
          {/* Tool calls */}
          {toolCalls.length > 0 && (
            <div className="flex flex-wrap gap-1">
              {toolCalls.map((tool, i) => (
                <span
                  key={i}
                  className="inline-flex items-center rounded bg-muted px-1.5 py-0.5 text-[10px]"
                >
                  {tool}
                </span>
              ))}
            </div>
          )}

          {/* Stream output */}
          {streamText && (
            <div className="max-h-64 overflow-y-auto rounded bg-muted/30 p-2 text-xs">
              <MarkdownRenderer content={streamText} />
            </div>
          )}

          {/* Error */}
          {error && (
            <div className="rounded bg-red-500/10 p-2 text-red-500 text-xs">
              {error}
            </div>
          )}

          {/* Run ID */}
          <div className="text-[10px] text-muted-foreground">
            Run: {runId}
          </div>
        </div>
      )}
    </div>
  )
}
