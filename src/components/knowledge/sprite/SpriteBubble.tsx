import { useTranslation } from "react-i18next"
import { CornerDownLeft, X } from "lucide-react"

import { AgentAvatarBadge, type AgentSelectAgent } from "@/components/common/AgentSelectDisplay"
import { IconTip } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import type { SpriteSuggestion } from "@/types/knowledge"

/**
 * Transient "sprite" bubble shown above the composer (Phase 2). A purple,
 * softly-glowing "magic" card with the agent avatar, a category badge, the
 * suggestion, and dismiss / respond actions. A blurred aura behind it gently
 * breathes (the diffusing effect). Never persisted into the message stream.
 */
export default function SpriteBubble({
  suggestion,
  agent,
  onDismiss,
  onRespond,
}: {
  suggestion: SpriteSuggestion
  agent?: AgentSelectAgent | null
  onDismiss: () => void
  onRespond: (text: string) => void
}) {
  const { t } = useTranslation()
  const cat = suggestion.category

  return (
    <div className="relative mx-2 mb-2 animate-in fade-in-0 slide-in-from-bottom-1 duration-300">
      {/* Soft diffusing aura — a blurred purple halo that gently breathes. */}
      <div
        aria-hidden
        className="pointer-events-none absolute -inset-1 rounded-[1.5rem] bg-purple-500/20 blur-xl animate-pulse [animation-duration:3.5s]"
      />
      <div
        className={cn(
          "relative rounded-2xl border border-purple-400/40 p-2.5 pr-8",
          // Opaque card base + a very faint purple wash so the text stays
          // high-contrast — the purple identity lives in the border / glow /
          // aura, not in a heavy fill.
          "bg-card bg-gradient-to-br from-purple-500/[0.06] via-transparent to-fuchsia-500/[0.05]",
          "shadow-[0_0_28px_-6px] shadow-purple-500/40 dark:border-purple-400/30",
        )}
      >
        <div className="flex items-start gap-2">
          <AgentAvatarBadge
            agent={agent ?? null}
            size="sm"
            className="mt-0.5 ring-2 ring-purple-400/40"
          />
          <div className="min-w-0 flex-1">
            <span className="inline-block rounded-full bg-purple-500/15 px-1.5 py-0.5 text-[10px] font-medium leading-none text-purple-600 dark:text-purple-300">
              {t(`knowledge.sprite.category.${cat}`, cat)}
            </span>
            <p className="mt-1 max-h-48 overflow-y-auto whitespace-pre-wrap break-words text-[13px] leading-relaxed text-foreground/90">
              {suggestion.text}
            </p>
            <button
              type="button"
              onClick={() => onRespond(suggestion.text)}
              className="mt-1.5 inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[11px] font-medium text-purple-600 transition-colors hover:bg-purple-500/10 dark:text-purple-300"
            >
              <CornerDownLeft className="h-3 w-3" />
              {t("knowledge.sprite.respond", "Respond")}
            </button>
          </div>
        </div>
      </div>
      <IconTip label={t("knowledge.sprite.dismiss", "Dismiss")}>
        <button
          type="button"
          onClick={onDismiss}
          aria-label={t("knowledge.sprite.dismiss", "Dismiss")}
          className="absolute right-1.5 top-1.5 z-10 flex h-5 w-5 items-center justify-center rounded-full text-purple-500/70 transition-colors hover:bg-purple-500/15 hover:text-purple-600 dark:text-purple-300/70"
        >
          <X className="h-3 w-3" />
        </button>
      </IconTip>
    </div>
  )
}
