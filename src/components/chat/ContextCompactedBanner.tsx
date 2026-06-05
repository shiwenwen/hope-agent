import { useTranslation } from "react-i18next"
import { Archive, Loader2 } from "lucide-react"
import { cn } from "@/lib/utils"
import type { ContextCompactedEvent } from "@/types/chat"

/** Inline context-compaction banner — muted gray chip. Distinct from the
 *  amber failover family: compaction is a normal operational event, not a
 *  failure. Tier 0/1 micro-compactions are filtered at the backend persist
 *  layer (see chat_engine/persister.rs); this banner only sees Tier ≥ 2. */
export default function ContextCompactedBanner({ event }: { event: ContextCompactedEvent }) {
  const { t } = useTranslation()
  const tier = typeof event.tier_applied === "number" ? event.tier_applied : undefined
  const isRunning =
    event.description === "summarizing" || event.description === "emergency_compacting"
  const summarizedMsgs =
    typeof event.messages_to_summarize === "number" ? event.messages_to_summarize : undefined
  const affectedMsgs =
    typeof event.messages_affected === "number" && event.messages_affected > 0
      ? event.messages_affected
      : undefined
  const msgs =
    isRunning || tier === 3 ? summarizedMsgs ?? affectedMsgs : affectedMsgs ?? summarizedMsgs
  const Icon = isRunning ? Loader2 : Archive

  const subtitle = (() => {
    const parts: string[] = []
    if (typeof tier === "number") parts.push(t("chat.contextCompactedTier", { tier }))
    if (typeof msgs === "number") {
      parts.push(
        t(isRunning ? "chat.contextCompactingMsgs" : "chat.contextCompactedMsgs", {
          count: msgs,
        }),
      )
    }
    return parts.join(" · ")
  })()

  return (
    <div
      className={cn(
        "mb-1.5 inline-flex max-w-full items-center gap-1.5 rounded-full border px-2.5 py-1 text-[11px]",
        isRunning
          ? "border-blue-500/25 bg-blue-500/10 text-blue-700 dark:text-blue-300"
          : "border-border/60 bg-muted/40 text-muted-foreground",
      )}
    >
      <Icon className={cn("h-3 w-3 shrink-0 opacity-70", isRunning && "animate-spin")} />
      <span className="shrink-0 font-medium text-foreground/75">
        {t(isRunning ? "chat.contextCompactingTitle" : "chat.contextCompactedTitle")}
      </span>
      {subtitle && (
        <>
          <span className="shrink-0 opacity-30">·</span>
          <span className="truncate opacity-70">{subtitle}</span>
        </>
      )}
    </div>
  )
}
