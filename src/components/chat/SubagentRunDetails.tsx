import { useState } from "react"
import { Check, Copy } from "lucide-react"
import { useTranslation } from "react-i18next"
import { IconTip } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"

interface DetailRow {
  key: string
  label: string
  value: string | number | undefined
  monospace?: boolean
}

interface SubagentRunDetailsProps {
  runId: string
  agentId: string
  childSessionId?: string
  task: string
  modelUsed?: string
  durationMs?: number
  inputTokens?: number
  outputTokens?: number
}

export default function SubagentRunDetails({
  runId,
  agentId,
  childSessionId,
  task,
  modelUsed,
  durationMs,
  inputTokens,
  outputTokens,
}: SubagentRunDetailsProps) {
  const { t } = useTranslation()
  const [copiedKey, setCopiedKey] = useState<string | null>(null)
  const tokenSummary =
    inputTokens != null && outputTokens != null
      ? `${inputTokens.toLocaleString()} in / ${outputTokens.toLocaleString()} out`
      : undefined
  const rows: DetailRow[] = [
    {
      key: "runId",
      label: t("subagent.runId", { defaultValue: "Run ID" }),
      value: runId,
      monospace: true,
    },
    {
      key: "agentId",
      label: t("subagent.agentId", { defaultValue: "Agent ID" }),
      value: agentId,
      monospace: true,
    },
    {
      key: "childSessionId",
      label: t("subagent.childSessionId", { defaultValue: "Child session" }),
      value: childSessionId,
      monospace: true,
    },
    {
      key: "task",
      label: t("subagent.task", { defaultValue: "Task" }),
      value: task,
    },
    {
      key: "model",
      label: t("subagent.model", { defaultValue: "Model" }),
      value: modelUsed,
      monospace: true,
    },
    {
      key: "duration",
      label: t("subagent.duration", { defaultValue: "Duration" }),
      value: durationMs != null ? `${(durationMs / 1000).toFixed(1)}s` : undefined,
    },
    {
      key: "tokens",
      label: t("subagent.tokens", { defaultValue: "Tokens" }),
      value: tokenSummary,
    },
  ].filter((row) => row.value !== undefined && row.value !== "")

  async function copyValue(key: string, value: string) {
    try {
      await navigator.clipboard?.writeText(value)
      setCopiedKey(key)
      window.setTimeout(() => setCopiedKey((cur) => (cur === key ? null : cur)), 1200)
    } catch {
      /* Clipboard is best-effort; details remain selectable. */
    }
  }

  return (
    <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1.5 rounded bg-background/70 px-2.5 py-2 text-[11px] leading-relaxed">
      {rows.map((row) => {
        const value = String(row.value)
        const copied = copiedKey === row.key
        return (
          <div key={row.key} className="contents">
            <dt className="text-muted-foreground whitespace-nowrap">{row.label}</dt>
            <dd className="min-w-0 flex items-center gap-1.5 text-foreground/85">
              <span
                className={cn(
                  "min-w-0 truncate select-text",
                  row.monospace && "font-mono text-[10px]",
                )}
                title={value}
              >
                {value}
              </span>
              <IconTip label={copied ? t("chat.copied") : t("chat.copy")}>
                <button
                  type="button"
                  className="shrink-0 rounded p-0.5 text-muted-foreground hover:bg-secondary hover:text-foreground transition-colors"
                  onClick={() => copyValue(row.key, value)}
                  aria-label={copied ? t("chat.copied") : t("chat.copy")}
                >
                  {copied ? <Check className="h-3 w-3" /> : <Copy className="h-3 w-3" />}
                </button>
              </IconTip>
            </dd>
          </div>
        )
      })}
    </dl>
  )
}
