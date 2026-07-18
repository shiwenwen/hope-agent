import { useState } from "react"
import { Check, Copy } from "lucide-react"
import { useTranslation } from "react-i18next"
import { IconTip } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import type { SessionMeta, SubagentRun } from "@/types/chat"
import { splitModelRef } from "./subagentShared"

interface DetailRow {
  key: string
  label: string
  value: string | number | undefined
  monospace?: boolean
}

function formatTimestamp(value: string | undefined, locale: string): string | undefined {
  if (!value) return undefined
  const parsed = new Date(value)
  return Number.isNaN(parsed.getTime()) ? value : parsed.toLocaleString(locale)
}

/** Full record for one sub-agent run — the "Details" pane of the sub-agent panel.
 *  Ordered meaning-first (what it did, how long, with what) and identifiers last.
 *
 *  `sessionMeta` is the run's CHILD session: `modelUsed` only encodes an opaque
 *  `<providerId>::<modelId>`, so the human provider name and the Think level
 *  come from there. */
export default function SubagentRunDetails({
  run,
  sessionMeta,
}: {
  run: SubagentRun
  sessionMeta?: SessionMeta | null
}) {
  const { t, i18n } = useTranslation()
  const [copiedKey, setCopiedKey] = useState<string | null>(null)

  const tokenSummary =
    run.inputTokens != null && run.outputTokens != null
      ? `${run.inputTokens.toLocaleString()} in / ${run.outputTokens.toLocaleString()} out`
      : undefined
  const { providerId, modelId } = splitModelRef(run.modelUsed)

  const rows: DetailRow[] = [
    {
      key: "label",
      label: t("subagent.label", { defaultValue: "Label" }),
      value: run.label,
    },
    {
      key: "task",
      label: t("subagent.task", { defaultValue: "Task" }),
      value: run.task,
    },
    {
      key: "provider",
      label: t("subagent.provider", { defaultValue: "Provider" }),
      // Prefer the resolved display name; fall back to the raw id only when the
      // child session meta hasn't loaded.
      value: sessionMeta?.providerName || providerId,
      monospace: !sessionMeta?.providerName,
    },
    {
      key: "model",
      label: t("subagent.model", { defaultValue: "Model" }),
      value: modelId || sessionMeta?.modelId || undefined,
      monospace: true,
    },
    {
      key: "thinking",
      label: t("subagent.thinking", { defaultValue: "Thinking" }),
      value: sessionMeta?.reasoningEffort || undefined,
    },
    {
      key: "duration",
      label: t("subagent.duration", { defaultValue: "Duration" }),
      value: run.durationMs != null ? `${(run.durationMs / 1000).toFixed(1)}s` : undefined,
    },
    {
      key: "tokens",
      label: t("subagent.tokens", { defaultValue: "Tokens" }),
      value: tokenSummary,
    },
    {
      key: "attachments",
      label: t("subagent.attachments", { defaultValue: "Attachments" }),
      // Zero attachments is the norm — only worth a row when there were some.
      value: run.attachmentCount ? run.attachmentCount : undefined,
    },
    {
      key: "startedAt",
      label: t("subagent.startedAt", { defaultValue: "Started" }),
      value: formatTimestamp(run.startedAt, i18n.language),
    },
    {
      key: "finishedAt",
      label: t("subagent.finishedAt", { defaultValue: "Finished" }),
      value: formatTimestamp(run.finishedAt, i18n.language),
    },
    {
      key: "depth",
      label: t("subagent.depth", { defaultValue: "Depth" }),
      value: run.depth,
    },
    {
      key: "agentId",
      label: t("subagent.agentId", { defaultValue: "Agent ID" }),
      value: run.childAgentId,
      monospace: true,
    },
    {
      key: "parentAgent",
      label: t("subagent.parentAgent", { defaultValue: "Parent agent" }),
      value: run.parentAgentId,
      monospace: true,
    },
    {
      key: "runId",
      label: t("subagent.runId", { defaultValue: "Run ID" }),
      value: run.runId,
      monospace: true,
    },
    {
      key: "childSessionId",
      label: t("subagent.childSessionId", { defaultValue: "Child session" }),
      value: run.childSessionId,
      monospace: true,
    },
    {
      key: "parentSession",
      label: t("subagent.parentSession", { defaultValue: "Parent session" }),
      value: run.parentSessionId,
      monospace: true,
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
    // No surface of its own — the hosting pane card provides it.
    <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1.5 text-[11px] leading-relaxed">
      {rows.map((row) => {
        const value = String(row.value)
        const copied = copiedKey === row.key
        return (
          <div key={row.key} className="contents">
            <dt className="whitespace-nowrap text-muted-foreground">{row.label}</dt>
            <dd className="flex min-w-0 items-center gap-1.5 text-foreground/85">
              <span
                className={cn(
                  "min-w-0 truncate select-text",
                  row.monospace && "font-mono text-[10px]",
                )}
                data-ha-title-tip={value}
              >
                {value}
              </span>
              <IconTip label={copied ? t("chat.copied") : t("chat.copy")}>
                <button
                  type="button"
                  className="shrink-0 rounded p-0.5 text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
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
