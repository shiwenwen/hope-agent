import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Sparkles } from "lucide-react"

import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"
// `knowledge_embedding_get_cmd` serializes to the generic EmbeddingSelectionState
// (same wire shape as memory's) — reuse the TS type.
import type { MemoryEmbeddingState as EmbeddingSelectionState } from "@/types/embedding-models"

/**
 * Title-bar status chip for knowledge vector search. Replaces the plain settings
 * gear: shows the active embedding model name when vector search is on, or an
 * "off" hint otherwise. Clicking always opens Settings → Knowledge — where the
 * user enables / picks / downloads a model (the activation dialog collapses to a
 * "go configure" CTA into the shared embedding-model library when none exist).
 *
 * Stays fresh by reloading on `config:changed` (enable / disable / model switch
 * and the post-reembed signature write all emit it).
 */
export default function KnowledgeEmbeddingBadge({
  onOpenSettings,
}: {
  onOpenSettings: () => void
}) {
  const { t } = useTranslation()
  const [state, setState] = useState<EmbeddingSelectionState | null>(null)

  useEffect(() => {
    const tx = getTransport()
    let cancelled = false
    const load = () => {
      tx.call<EmbeddingSelectionState>("knowledge_embedding_get_cmd")
        .then((st) => {
          if (cancelled) return
          // Skip the re-render when nothing observable changed (config:changed
          // fires for unrelated categories too).
          setState((prev) =>
            prev &&
            prev.selection.enabled === st.selection.enabled &&
            (prev.currentModel?.id ?? null) === (st.currentModel?.id ?? null) &&
            (prev.currentModel?.name ?? null) === (st.currentModel?.name ?? null)
              ? prev
              : st,
          )
        })
        .catch((e) =>
          logger.warn("knowledge", "KnowledgeEmbeddingBadge::load", "Failed to load state", e),
        )
    }
    load()
    const unlisten = tx.listen("config:changed", load)
    return () => {
      cancelled = true
      unlisten()
    }
  }, [])

  const on = !!state?.selection.enabled && !!state.currentModel
  const label = on
    ? state!.currentModel!.name
    : t("knowledge.embeddingOff", "Vector search off")

  return (
    <IconTip
      label={
        on
          ? t("knowledge.embeddingModelTip", {
              name: state!.currentModel!.name,
              defaultValue: "Embedding model: {{name}} — open settings",
            })
          : t("knowledge.embeddingOffTip", "Vector search is off — open settings")
      }
      side="bottom"
    >
      <Button
        variant="ghost"
        size="sm"
        className="h-8 max-w-[180px] gap-1.5 px-2"
        onClick={onOpenSettings}
      >
        <Sparkles
          className={cn("h-3.5 w-3.5 shrink-0", on ? "text-primary" : "text-muted-foreground")}
        />
        <span className={cn("truncate text-xs", !on && "text-muted-foreground")}>{label}</span>
      </Button>
    </IconTip>
  )
}
