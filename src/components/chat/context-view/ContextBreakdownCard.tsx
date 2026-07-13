import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { cn } from "@/lib/utils"
import { IconTip } from "@/components/ui/tooltip"
import { Loader2, Zap, FileText, Eye } from "lucide-react"
import { logger } from "@/lib/logger"
import type { ContextBreakdown } from "@/components/chat/slash-commands/types"
import {
  type CompactResult,
  compactContextNow,
  compactResultMessage,
} from "@/components/chat/sessionStatus"

interface ContextBreakdownCardProps {
  data: ContextBreakdown
  sessionId?: string | null
  compacting?: boolean
  onCompactContext?: () => Promise<CompactResult | null>
  onCompactDone?: () => void
  onViewSystemPrompt?: () => void
}

type Segment = {
  key: string
  label: string
  tokens: number
  color: string
  dotColor: string
}

function formatK(tokens: number): string {
  if (tokens >= 1000) {
    return `${(tokens / 1000).toFixed(1)}k`
  }
  return `${tokens}`
}

function formatDuration(secs: number): string {
  if (secs < 60) return `${secs}s`
  if (secs < 3600) return `${Math.floor(secs / 60)}m`
  return `${Math.floor(secs / 3600)}h`
}

export default function ContextBreakdownCard({
  data,
  sessionId,
  compacting: externalCompacting = false,
  onCompactContext,
  onCompactDone,
  onViewSystemPrompt,
}: ContextBreakdownCardProps) {
  const { t } = useTranslation()
  const [compacting, setCompacting] = useState(false)
  const [cooldown, setCooldown] = useState<number | null>(
    data.nextCompactAllowedInSecs ?? null,
  )

  useEffect(() => {
    setCooldown(data.nextCompactAllowedInSecs ?? null)
  }, [data])

  // Tick cooldown countdown once per second.
  useEffect(() => {
    if (cooldown == null || cooldown <= 0) return
    const id = setInterval(() => {
      setCooldown((c) => (c != null && c > 0 ? c - 1 : c))
    }, 1000)
    return () => clearInterval(id)
  }, [cooldown])

  const segments: Segment[] = [
    {
      key: "systemPrompt",
      label: t("context.systemPrompt", "System prompt"),
      tokens: data.systemPromptTokens,
      color: "bg-blue-500/70",
      dotColor: "bg-blue-500",
    },
    {
      key: "toolSchemas",
      label: t("context.toolSchemas", "Tool schemas"),
      tokens: data.toolSchemasTokens,
      color: "bg-violet-500/70",
      dotColor: "bg-violet-500",
    },
    {
      key: "toolDescriptions",
      label: t("context.toolDescriptions", "Tool descriptions"),
      tokens: data.toolDescriptionsTokens,
      color: "bg-purple-500/70",
      dotColor: "bg-purple-500",
    },
    {
      key: "memory",
      label: t("context.memory", "Memory"),
      tokens: data.memoryTokens,
      color: "bg-green-500/70",
      dotColor: "bg-green-500",
    },
    {
      key: "dynamicPrompt",
      label: t("context.dynamicPrompt", "Dynamic prompt"),
      tokens: data.dynamicPromptTokens ?? 0,
      color: "bg-cyan-500/70",
      dotColor: "bg-cyan-500",
    },
    {
      key: "skills",
      label: t("context.skills", "Skills"),
      tokens: data.skillTokens,
      color: "bg-amber-500/70",
      dotColor: "bg-amber-500",
    },
    {
      key: "messages",
      label: t("context.messages", "Messages"),
      tokens: data.messagesTokens,
      color: "bg-slate-500/70",
      dotColor: "bg-slate-500",
    },
    {
      key: "reservedOutput",
      label: t("context.reservedOutput", "Reserved output"),
      tokens: data.maxOutputTokens,
      color: "bg-rose-500/60",
      dotColor: "bg-rose-500",
    },
    {
      key: "freeSpace",
      label: t("context.freeSpace", "Free space"),
      tokens: data.freeSpace,
      color: "bg-muted",
      dotColor: "bg-muted-foreground/40",
    },
  ]

  const totalForBar = data.contextWindow > 0 ? data.contextWindow : 1
  const pct = Math.round(data.usagePct)
  const pctColor =
    pct < 50 ? "text-green-500" : pct < 80 ? "text-yellow-500" : "text-red-500"
  const compactBusy = externalCompacting || compacting
  const canCompact = !!sessionId && (cooldown == null || cooldown <= 0) && !compactBusy
  const cacheReadRatio =
    data.cacheReadTokens != null && (data.cacheableStableTokensEstimate ?? 0) > 0
      ? Math.min(100, Math.round((data.cacheReadTokens / data.cacheableStableTokensEstimate!) * 100))
      : null

  const handleCompact = async () => {
    if (!sessionId) return
    setCompacting(true)
    try {
      const result = onCompactContext
        ? await onCompactContext()
        : await compactContextNow(sessionId)
      if (result) {
        toast.success(compactResultMessage(t, result))
        onCompactDone?.()
      }
    } catch (e) {
      logger.error("ui", "ContextBreakdownCard::compact", "Compact failed", e)
      toast.error(t("chat.compactFailed"))
    } finally {
      setCompacting(false)
    }
  }

  return (
    <div className="w-full max-w-2xl rounded-xl border border-border bg-card shadow-sm overflow-hidden">
      {/* ── Header ─────────────────────────────────────────── */}
      <div className="px-4 py-3 border-b border-border bg-muted/30 flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 min-w-0">
          <Zap className="w-4 h-4 shrink-0 text-foreground/70" />
          <span className="text-sm font-medium text-foreground truncate">
            {t("context.title", "Context window")}
          </span>
        </div>
        <div className="flex items-center gap-2 text-xs text-muted-foreground shrink-0">
          <IconTip label={`${data.activeProvider} / ${data.activeModel}`}>
            <span className="truncate max-w-[200px]">{data.activeModel}</span>
          </IconTip>
        </div>
      </div>

      {/* ── Total usage ─────────────────────────────────────── */}
      <div className="px-4 pt-3">
        <div className="flex items-baseline justify-between gap-2 mb-1.5">
          <span className="text-xs text-muted-foreground">
            {t("context.used", "Used")}
          </span>
          <div className="flex items-baseline gap-1 tabular-nums">
            <span className="text-base font-semibold text-foreground">
              {formatK(data.usedTotal)}
            </span>
            <span className="text-xs text-muted-foreground">
              / {formatK(data.contextWindow)}
            </span>
            <span className={cn("text-xs font-medium ml-1", pctColor)}>({pct}%)</span>
          </div>
        </div>

        {/* Segmented bar */}
        <div className="h-2.5 w-full rounded-full overflow-hidden flex bg-muted">
          {segments.map((seg) => {
            const segPct = (seg.tokens / totalForBar) * 100
            if (segPct <= 0) return null
            return (
              <IconTip
                key={seg.key}
                label={`${seg.label} — ${formatK(seg.tokens)} (${Math.round(segPct)}%)`}
              >
                <div
                  className={cn("h-full transition-all", seg.color)}
                  style={{ width: `${segPct}%` }}
                />
              </IconTip>
            )
          })}
        </div>
      </div>

      {/* ── Per-category rows ───────────────────────────────── */}
      <div className="px-4 py-3 space-y-1">
        {segments.map((seg) => {
          const segPct = totalForBar > 0 ? (seg.tokens / totalForBar) * 100 : 0
          return (
            <div
              key={seg.key}
              className="flex items-center justify-between text-xs py-0.5"
            >
              <div className="flex items-center gap-2 min-w-0">
                <span className={cn("w-1.5 h-1.5 rounded-full shrink-0", seg.dotColor)} />
                <span className="text-muted-foreground truncate">{seg.label}</span>
              </div>
              <div className="tabular-nums text-foreground/80 shrink-0">
                {formatK(seg.tokens)}{" "}
                <span className="text-muted-foreground">({Math.round(segPct)}%)</span>
              </div>
            </div>
          )
        })}
      </div>

      {(data.contextInputTokens != null || data.requestInputTokensEstimate != null) && (
        <div className="border-t border-border/60 px-4 py-3 text-[11px] text-muted-foreground">
          <div className="flex flex-wrap gap-x-4 gap-y-1">
            <span>
              {t("context.providerInput", "Provider input")}: {formatK(
                data.contextInputTokens ?? data.requestInputTokensEstimate ?? 0,
              )}
              {data.contextInputTokens == null && ` ${t("context.estimated", "estimated")}`}
            </span>
            {data.freshInputTokens != null && (
              <span>{t("context.freshInput", "Fresh input")}: {formatK(data.freshInputTokens)}</span>
            )}
            {data.cacheReadTokens != null && (
              <span>
                {t("context.cacheRead", "Cache read")}: {formatK(data.cacheReadTokens)}
                {cacheReadRatio != null && ` · ${cacheReadRatio}%`}
              </span>
            )}
            {data.cacheWriteTokens != null && data.cacheWriteTokens > 0 && (
              <span>{t("context.cacheWrite", "Cache write")}: {formatK(data.cacheWriteTokens)}</span>
            )}
            {data.ttftMs != null && <span>TTFT: {data.ttftMs}ms</span>}
          </div>
          <div className="mt-1 opacity-70">
            {t(
              "context.cacheWindowNotice",
              "Cache hits reduce repeated computation, cost, and TTFT; they do not reduce context-window usage.",
            )}
          </div>
        </div>
      )}

      {/* ── Compaction status ───────────────────────────────── */}
      {(data.lastCompactTier != null || cooldown != null) && (
        <div className="px-4 pb-2 text-[11px] text-muted-foreground space-y-0.5">
          {data.lastCompactTier != null && data.lastCompactSecsAgo != null && (
            <div>
              {t("context.lastCompact", "Last compact")}: Tier {data.lastCompactTier} ·{" "}
              {formatDuration(data.lastCompactSecsAgo)}{" "}
              {t("context.ago", "ago")}
            </div>
          )}
          {cooldown != null && cooldown > 0 && (
            <div>
              {t("context.compactAllowedIn", "Next compact in")}: {cooldown}s{" "}
              <span className="opacity-60">
                ({t("context.cacheTtlProtection", "cache TTL protection")})
              </span>
            </div>
          )}
        </div>
      )}

      {/* ── Actions ─────────────────────────────────────────── */}
      <div className="px-4 py-3 border-t border-border flex items-center gap-2">
        <button
          onClick={handleCompact}
          disabled={!canCompact}
          className="flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md border border-border/60 hover:bg-secondary/60 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
        >
          {compactBusy ? (
            <Loader2 className="w-3.5 h-3.5 animate-spin" />
          ) : (
            <Zap className="w-3.5 h-3.5" />
          )}
          {compactBusy
            ? t("chat.compacting", "Compacting…")
            : t("chat.compactNow", "Compact now")}
        </button>
        <button
          onClick={onViewSystemPrompt}
          className="flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md border border-border/60 hover:bg-secondary/60 transition-colors"
        >
          <FileText className="w-3.5 h-3.5" />
          {t("context.viewSystemPrompt", "System prompt")}
        </button>
        <div className="flex-1" />
        <IconTip
          label={t(
            "context.estimateNote",
            "Provider usage is authoritative after each round; before that, a conservative request-shape estimate is shown.",
          )}
        >
          <Eye className="w-3.5 h-3.5 text-muted-foreground/70" />
        </IconTip>
      </div>
    </div>
  )
}
