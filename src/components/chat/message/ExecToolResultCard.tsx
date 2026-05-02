import { useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { XCircle } from "lucide-react"
import type { ToolCall } from "@/types/chat"
import { getTransport } from "@/lib/transport-provider"
import { IconTip } from "@/components/ui/tooltip"

function parseExecCommand(tool: ToolCall): string {
  try {
    const parsed = JSON.parse(tool.arguments) as { command?: string }
    return parsed.command?.trim() || ""
  } catch {
    return ""
  }
}

function getDisplayOutput(result: string | undefined): string | null {
  if (!result) return null
  const trimmed = result.trim()
  if (trimmed === "Command completed with exit code 0") return null
  return result
}

function parseBackgroundSessionId(result: string | undefined): string | null {
  if (!result) return null
  if (result.includes("Process exited") || result.includes("Terminated session")) return null
  const match = result.match(/session ([^\s)]+)\)/)
  return match?.[1] ?? null
}

export default function ExecToolResultCard({
  tool,
  isRunning,
}: {
  tool: ToolCall
  isRunning: boolean
}) {
  const { t } = useTranslation()
  const [cancelled, setCancelled] = useState(false)
  const command = parseExecCommand(tool)
  const output = getDisplayOutput(tool.result)
  const backgroundSessionId = parseBackgroundSessionId(tool.result)
  const outputRef = useRef<HTMLPreElement>(null)

  useEffect(() => {
    const el = outputRef.current
    if (!el) return
    el.scrollTop = el.scrollHeight
  }, [output, isRunning])

  async function cancelProcess() {
    if (!backgroundSessionId || cancelled) return
    setCancelled(true)
    try {
      await getTransport().call("cancel_runtime_task", { kind: "process", id: backgroundSessionId })
    } catch {
      setCancelled(false)
    }
  }

  return (
    <div className="rounded-lg border border-border/50 bg-secondary/40 px-3 py-2.5">
      <div className="mb-2 flex items-center gap-2">
        <div className="text-[11px] font-semibold text-muted-foreground/80">
          {t("tools.execPanel.title", "Shell")}
        </div>
        {backgroundSessionId && !cancelled && (
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
        {output ? (
          <pre
            ref={outputRef}
            className="whitespace-pre-wrap break-words text-muted-foreground/85 font-mono text-[11px] leading-relaxed max-h-64 overflow-y-auto"
          >
            {output}
          </pre>
        ) : isRunning ? (
          <div className="text-[11px] text-muted-foreground/60">
            {t("tools.execPanel.running", "运行中...")}
          </div>
        ) : (
          <div className="text-[11px] text-muted-foreground/60">
            {t("tools.execPanel.noOutput", "无输出")}
          </div>
        )}
      </div>
    </div>
  )
}
