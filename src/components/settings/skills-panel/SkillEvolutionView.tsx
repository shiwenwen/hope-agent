import { useTranslation } from "react-i18next"
import { Sparkles } from "lucide-react"
import { Switch } from "@/components/ui/switch"
import { cn } from "@/lib/utils"

interface SkillEvolutionViewProps {
  autoReviewEnabled: boolean
  autoReviewPromotion: boolean
  onSetAutoReviewEnabled: (v: boolean) => void
  onSetAutoReviewPromotion: (v: boolean) => void
}

export default function SkillEvolutionView({
  autoReviewEnabled,
  autoReviewPromotion,
  onSetAutoReviewEnabled,
  onSetAutoReviewPromotion,
}: SkillEvolutionViewProps) {
  const { t } = useTranslation()

  return (
    <div className="flex-1 min-h-0 overflow-y-auto p-6 space-y-5">
      {/* Hero: master switch */}
      <div className="overflow-hidden rounded-2xl border border-violet-500/25 bg-gradient-to-br from-violet-500/10 via-fuchsia-500/8 to-pink-500/5 dark:border-violet-400/30 dark:from-violet-500/15 dark:via-fuchsia-500/12 dark:to-pink-500/8">
        <div className="flex items-start gap-4 p-6">
          <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-2xl bg-gradient-to-br from-violet-500 to-fuchsia-500 shadow-lg shadow-violet-500/30">
            <Sparkles className="h-5 w-5 text-white" />
          </div>
          <div className="flex-1 min-w-0">
            <div className="flex flex-wrap items-center gap-2 mb-1.5">
              <h3 className="text-base font-semibold text-foreground">
                {t("settings.skillsEvolutionHero.title")}
              </h3>
              <span
                className={cn(
                  "inline-flex items-center gap-1.5 text-[10px] px-2 py-0.5 rounded-full font-medium",
                  autoReviewEnabled
                    ? "bg-emerald-500/15 text-emerald-700 dark:text-emerald-400"
                    : "bg-muted text-muted-foreground",
                )}
              >
                <span
                  className={cn(
                    "h-1.5 w-1.5 rounded-full",
                    autoReviewEnabled
                      ? "bg-emerald-500 animate-pulse"
                      : "bg-muted-foreground/40",
                  )}
                />
                {autoReviewEnabled
                  ? t("settings.skillsEvolutionHero.statusOn")
                  : t("settings.skillsEvolutionHero.statusOff")}
              </span>
            </div>
            <p className="text-sm leading-relaxed text-muted-foreground">
              {t("settings.skillsEvolutionHero.body")}
            </p>
            <p className="mt-2 text-xs text-muted-foreground/80">
              {t("settings.skillsEvolutionHero.note")}
            </p>
          </div>
          <Switch
            checked={autoReviewEnabled}
            onCheckedChange={onSetAutoReviewEnabled}
            className="mt-1 shrink-0 data-[state=checked]:bg-gradient-to-r data-[state=checked]:from-violet-500 data-[state=checked]:to-fuchsia-500"
          />
        </div>
      </div>

      {/* Promotion toggle (gated by master switch) */}
      <div
        className={cn(
          "rounded-xl border border-border bg-card/50 p-4 transition-opacity",
          !autoReviewEnabled && "opacity-50",
        )}
      >
        <div className="flex items-start justify-between gap-4">
          <div className="min-w-0">
            <div className="text-sm font-medium text-foreground">
              {t("settings.skillsAutoReview.label")}
            </div>
            <div className="mt-0.5 text-xs text-muted-foreground">
              {t("settings.skillsAutoReview.description")}
            </div>
            <div className="mt-0.5 text-xs text-muted-foreground/70">
              {t("settings.skillsAutoReview.hint")}
            </div>
          </div>
          <Switch
            checked={autoReviewPromotion}
            onCheckedChange={onSetAutoReviewPromotion}
            disabled={!autoReviewEnabled}
            className="shrink-0"
          />
        </div>
      </div>
    </div>
  )
}
