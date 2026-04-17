import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Check, Loader2, Sparkles, Trash2 } from "lucide-react"
import type { SkillSummary } from "../types"

interface DraftReviewSectionProps {
  drafts: SkillSummary[]
  pendingAction: Record<string, "activate" | "discard" | undefined>
  onActivate: (name: string) => void
  onDiscard: (name: string) => void
  onSelectSkill: (name: string) => void
}

/**
 * Top-of-panel card listing skills in `status: draft` (auto-created by the
 * Phase B' review pipeline, awaiting human promotion). Only renders when
 * there's at least one draft.
 */
export default function DraftReviewSection({
  drafts,
  pendingAction,
  onActivate,
  onDiscard,
  onSelectSkill,
}: DraftReviewSectionProps) {
  const { t } = useTranslation()
  if (drafts.length === 0) return null

  return (
    <div className="mb-5 rounded-lg border border-amber-500/40 bg-amber-500/5 p-3">
      <div className="flex items-center gap-2 mb-2">
        <Sparkles className="h-4 w-4 text-amber-500" />
        <h3 className="text-sm font-semibold text-foreground">
          {t("settings.skillsDraftsTitle")}
        </h3>
        <span className="text-xs text-amber-600 dark:text-amber-400 font-medium ml-1">
          ({drafts.length})
        </span>
      </div>
      <p className="text-xs text-muted-foreground mb-3">
        {t("settings.skillsDraftsDesc")}
      </p>
      <div className="space-y-1.5">
        {drafts.map((d) => {
          const pending = pendingAction[d.name]
          return (
            <div
              key={d.name}
              className="flex items-center gap-2 p-2 rounded-md bg-background/60 border border-border/40"
            >
              <button
                className="flex-1 text-left min-w-0 hover:text-foreground transition-colors"
                onClick={() => onSelectSkill(d.name)}
              >
                <div className="text-sm font-medium truncate">{d.name}</div>
                <div className="text-xs text-muted-foreground truncate">{d.description}</div>
                {d.authored_by && (
                  <div className="text-[10px] text-muted-foreground/70 mt-0.5">
                    {t("settings.skillsDraftsAuthoredBy")}: {d.authored_by}
                  </div>
                )}
              </button>
              <Button
                size="sm"
                variant="default"
                className="h-7 px-2.5"
                onClick={() => onActivate(d.name)}
                disabled={!!pending}
              >
                {pending === "activate" ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Check className="h-3.5 w-3.5 mr-1" />
                )}
                {pending !== "activate" && t("settings.skillsDraftsActivate")}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                className="h-7 px-2 text-destructive/80 hover:text-destructive hover:bg-destructive/10"
                onClick={() => onDiscard(d.name)}
                disabled={!!pending}
              >
                {pending === "discard" ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Trash2 className="h-3.5 w-3.5" />
                )}
              </Button>
            </div>
          )
        })}
      </div>
    </div>
  )
}
