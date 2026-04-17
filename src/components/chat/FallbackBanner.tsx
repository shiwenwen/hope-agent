import { useState, useRef, useEffect } from "react"
import { useTranslation } from "react-i18next"
import { AlertTriangle, ArrowRight, ChevronDown } from "lucide-react"
import ProviderIcon from "@/components/common/ProviderIcon"
import FallbackDetailsPopover from "@/components/chat/FallbackDetailsPopover"
import { cn } from "@/lib/utils"
import type { FallbackEvent } from "@/types/chat"

interface ModelParts {
  provider: string
  model: string
}

/** Parse backend-provided "providerName / modelId" display string. */
function parseModel(display?: string): ModelParts | null {
  if (!display) return null
  const idx = display.indexOf(" / ")
  if (idx < 0) return { provider: "", model: display }
  return { provider: display.slice(0, idx), model: display.slice(idx + 3) }
}

function ModelChip({
  parts,
  providerKey,
  dim,
}: {
  parts: ModelParts
  providerKey?: string
  dim?: boolean
}) {
  return (
    <span
      className={cn(
        "inline-flex min-w-0 items-center gap-1",
        dim && "opacity-60",
      )}
    >
      <ProviderIcon
        providerKey={providerKey}
        providerName={parts.provider || undefined}
        size={12}
        color
        className="shrink-0"
      />
      <span className={cn("truncate", dim ? "font-normal" : "font-semibold text-foreground/90")}>
        {parts.model}
      </span>
    </span>
  )
}

/** Inline fallback banner — compact amber chip, click to expand details. */
export default function FallbackBanner({ event }: { event: FallbackEvent }) {
  const { t } = useTranslation()
  const [showPopover, setShowPopover] = useState(false)
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!showPopover) return
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setShowPopover(false)
      }
    }
    document.addEventListener("mousedown", handler)
    return () => document.removeEventListener("mousedown", handler)
  }, [showPopover])

  const toParts = parseModel(event.model) ?? { provider: "", model: event.model }
  const fromParts = parseModel(event.from_model)

  return (
    <div ref={ref} className="relative mb-1.5 inline-block max-w-full">
      <button
        type="button"
        onClick={() => setShowPopover((v) => !v)}
        className={cn(
          "group inline-flex max-w-full items-center gap-1.5 rounded-full border px-2.5 py-1 text-[11px] transition-colors",
          "border-amber-500/25 bg-amber-500/[0.07] text-muted-foreground",
          "hover:bg-amber-500/[0.12] hover:border-amber-500/40",
        )}
      >
        <AlertTriangle className="h-3 w-3 shrink-0 text-amber-500" />
        <span className="shrink-0 font-medium text-foreground/75">
          {t("chat.fallbackTitle")}
        </span>
        <span className="shrink-0 opacity-30">·</span>
        {fromParts && (
          <>
            <ModelChip parts={fromParts} dim />
            <ArrowRight className="h-3 w-3 shrink-0 opacity-40" />
          </>
        )}
        <ModelChip parts={toParts} providerKey={event.provider_id} />
        {event.attempt != null && event.total != null && (
          <span className="shrink-0 tabular-nums text-[10px] opacity-55">
            {event.attempt}/{event.total}
          </span>
        )}
        <ChevronDown
          className={cn(
            "h-3 w-3 shrink-0 opacity-40 transition-transform",
            showPopover && "rotate-180",
          )}
        />
      </button>
      <FallbackDetailsPopover event={event} open={showPopover} />
    </div>
  )
}
