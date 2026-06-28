import { useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { XCircle } from "lucide-react"
import type { ToolCall } from "@/types/chat"
import { getTransport } from "@/lib/transport-provider"
import { IconTip } from "@/components/ui/tooltip"

const LIVE_OUTPUT_MAX_CHARS = 32_000

function parseExecCommand(tool: ToolCall): string {
  try {
    const parsed = JSON.parse(tool.arguments) as { command?: string }
    return parsed.command?.trim() || ""
  } catch {
    return ""
  }
}

function parseAsyncJobStarted(result: string | undefined): { jobId: string; tool?: string } | null {
  if (!result) return null
  try {
    const parsed = JSON.parse(result) as { job_id?: string; status?: string; tool?: string }
    if (parsed.status !== "started" || !parsed.job_id) return null
    return { jobId: parsed.job_id, tool: parsed.tool }
  } catch {
    return null
  }
}

function getDisplayOutput(result: string | undefined): string | null {
  if (!result) return null
  const trimmed = result.trim()
  if (trimmed === "Command completed with exit code 0") return null
  if (parseAsyncJobStarted(result)) return null
  return result
}

function parseBackgroundSessionId(result: string | undefined): string | null {
  if (!result) return null
  if (result.includes("Process exited") || result.includes("Terminated session")) return null
  const match = result.match(/session ([^\s)]+)\)/)
  return match?.[1] ?? null
}

function appendCapped(prev: string, chunk: string): string {
  const next = `${prev}${chunk}`
  if (next.length <= LIVE_OUTPUT_MAX_CHARS) return next
  return next.slice(next.length - LIVE_OUTPUT_MAX_CHARS)
}

function processExitLine(state: ProcessLiveState | null): string | null {
  if (!state?.terminal) return null
  const detail = state.exitSignal ? `signal ${state.exitSignal}` : `code ${state.exitCode ?? 0}`
  return `Process exited with ${detail}.`
}

interface ProcessLiveState {
  sessionId: string
  output: string
  status: string
  terminal: boolean
  exitCode?: number | null
  exitSignal?: string | null
}

export default function ExecToolResultCard({ tool, isRunning }: { tool: ToolCall; isRunning: boolean }) {
  const { t } = useTranslation()
  const [cancelledSessionId, setCancelledSessionId] = useState<string | null>(null)
  const [processLive, setProcessLive] = useState<ProcessLiveState | null>(null)
  const command = useMemo(() => parseExecCommand(tool), [tool])
  const asyncJob = useMemo(() => parseAsyncJobStarted(tool.result), [tool.result])
  const output = useMemo(() => getDisplayOutput(tool.result), [tool.result])
  const backgroundSessionId = useMemo(() => parseBackgroundSessionId(tool.result), [tool.result])
  const activeProcessLive = processLive?.sessionId === backgroundSessionId ? processLive : null
  const cancelled = cancelledSessionId === backgroundSessionId
  const exitLine = processExitLine(activeProcessLive)
  const displayOutput = useMemo(() => {
    const parts = [output, activeProcessLive?.output || null, exitLine].filter(Boolean)
    return parts.length ? parts.join("\n\n") : null
  }, [output, activeProcessLive?.output, exitLine])
  const outputRef = useRef<HTMLPreElement>(null)

  useEffect(() => {
    const el = outputRef.current
    if (!el) return
    el.scrollTop = el.scrollHeight
  }, [displayOutput, isRunning])

  useEffect(() => {
    if (!backgroundSessionId) return

    const offOutput = getTransport().listen("process:output", (raw) => {
      const payload = raw as {
        process_id?: string
        chunk?: string
        status?: string
      }
      if (payload.process_id !== backgroundSessionId || !payload.chunk) return
      setProcessLive((prev) => ({
        sessionId: backgroundSessionId,
        output: appendCapped(prev?.sessionId === backgroundSessionId ? prev.output : "", payload.chunk || ""),
        status: payload.status || prev?.status || "running",
        terminal: prev?.sessionId === backgroundSessionId ? prev.terminal : false,
        exitCode: prev?.sessionId === backgroundSessionId ? prev.exitCode : undefined,
        exitSignal: prev?.sessionId === backgroundSessionId ? prev.exitSignal : undefined,
      }))
    })
    const offCompleted = getTransport().listen("process:completed", (raw) => {
      const payload = raw as {
        process_id?: string
        status?: string
        exit_code?: number | null
        exit_signal?: string | null
        tail?: string
      }
      if (payload.process_id !== backgroundSessionId) return
      setProcessLive((prev) => ({
        sessionId: backgroundSessionId,
        output: prev?.sessionId === backgroundSessionId ? prev.output || payload.tail || "" : payload.tail || "",
        status: payload.status || prev?.status || "completed",
        terminal: true,
        exitCode: payload.exit_code ?? null,
        exitSignal: payload.exit_signal ?? null,
      }))
    })
    return () => {
      offOutput()
      offCompleted()
    }
  }, [backgroundSessionId])

  async function cancelProcess() {
    if (!backgroundSessionId || cancelled || activeProcessLive?.terminal) return
    setCancelledSessionId(backgroundSessionId)
    try {
      const result = await getTransport().call<{ status?: string }>("cancel_runtime_task", {
        kind: "process",
        id: backgroundSessionId,
      })
      if (result?.status) {
        setProcessLive((prev) => ({
          sessionId: backgroundSessionId,
          output: prev?.sessionId === backgroundSessionId ? prev.output : "",
          status: result.status || prev?.status || "killed",
          terminal: result.status !== "running" && result.status !== "cancelling",
          exitCode: prev?.sessionId === backgroundSessionId ? prev.exitCode : undefined,
          exitSignal: prev?.sessionId === backgroundSessionId ? prev.exitSignal ?? "SIGKILL" : "SIGKILL",
        }))
      }
    } catch {
      setCancelledSessionId((current) => (current === backgroundSessionId ? null : current))
    }
  }

  return (
    <div className="rounded-lg border border-border/50 bg-secondary/40 px-3 py-2.5">
      <div className="mb-2 flex items-center gap-2">
        <div className="text-[11px] font-semibold text-muted-foreground/80">
          {t("tools.execPanel.title", "Shell")}
        </div>
        {backgroundSessionId && activeProcessLive?.status && (
          <span className="rounded bg-secondary px-1.5 py-0.5 text-[10px] text-muted-foreground">
            {activeProcessLive.status}
          </span>
        )}
        {backgroundSessionId && !cancelled && !activeProcessLive?.terminal && (
          <IconTip label={t("common.cancel")}>
            <button
              type="button"
              className="ml-auto rounded p-0.5 text-muted-foreground/60 transition-colors hover:bg-secondary hover:text-red-500"
              onClick={cancelProcess}
              aria-label={t("common.cancel")}
            >
              <XCircle className="h-3 w-3" />
            </button>
          </IconTip>
        )}
      </div>
      <pre className="whitespace-pre-wrap break-all text-foreground font-mono text-xs leading-relaxed">
        $ {command}
      </pre>
      <div className="mt-2.5 border-t border-border/40 pt-2.5">
        {asyncJob ? (
          <div className="text-[11px] text-muted-foreground/70">
            {t("tools.execPanel.asyncStarted", "后台任务已启动")}:{" "}
            <span className="font-mono">{asyncJob.jobId}</span>
          </div>
        ) : displayOutput ? (
          <pre
            ref={outputRef}
            className="whitespace-pre-wrap break-words text-muted-foreground/85 font-mono text-[11px] leading-relaxed max-h-64 overflow-y-auto"
          >
            {displayOutput}
          </pre>
        ) : isRunning || (backgroundSessionId && !activeProcessLive?.terminal) ? (
          <div className="text-[11px] text-muted-foreground/60">{t("tools.execPanel.running", "运行中...")}</div>
        ) : (
          <div className="text-[11px] text-muted-foreground/60">{t("tools.execPanel.noOutput", "无输出")}</div>
        )}
      </div>
    </div>
  )
}
