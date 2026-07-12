import { Trans, useTranslation } from "react-i18next"
import { Bug, Hammer, RefreshCcw, Search, type LucideIcon } from "lucide-react"

import alphaLogoUrl from "@/assets/alpha-logo.png"
import { cn } from "@/lib/utils"

interface ProjectWelcomeSuggestion {
  key: "explore" | "build" | "review" | "fix"
  icon: LucideIcon
  iconClassName: string
}

const PROJECT_WELCOME_SUGGESTIONS: ProjectWelcomeSuggestion[] = [
  { key: "explore", icon: Search, iconClassName: "text-sky-500" },
  { key: "build", icon: Hammer, iconClassName: "text-violet-500" },
  { key: "review", icon: RefreshCcw, iconClassName: "text-emerald-500" },
  { key: "fix", icon: Bug, iconClassName: "text-orange-500" },
]

/**
 * The empty-session greeting (logo + slogan, or the incognito notice). Shared by
 * {@link MessageList} (plain centered empty state) and `ChatScreen`'s hero
 * composer, where it sits directly above the centered input as a single
 * vertically-centered unit — so the greeting and the composer can never overlap
 * regardless of width/height (the two used to center independently and collided
 * when the pane was squeezed).
 */
export function ChatWelcomeHero({
  incognito = false,
  projectName,
  onProjectSuggestion,
}: {
  incognito?: boolean
  projectName?: string | null
  onProjectSuggestion?: (prompt: string) => void
}) {
  const { t } = useTranslation()

  if (incognito) {
    return (
      <div className="mx-auto max-w-[680px] px-4 text-center">
        <img
          src={alphaLogoUrl}
          alt=""
          className="mx-auto mb-5 h-[72px] w-[72px] object-contain opacity-35 grayscale"
          draggable={false}
        />
        <p className="mx-auto max-w-[720px] text-2xl leading-relaxed text-foreground sm:text-3xl">
          {t("chat.incognitoEmptyBody")}
        </p>
      </div>
    )
  }

  if (projectName) {
    return (
      <div className="mx-auto w-full px-4 text-center">
        <img
          src={alphaLogoUrl}
          alt=""
          className="mx-auto mb-5 h-[72px] w-[72px] object-contain opacity-95"
          draggable={false}
        />
        <p className="mx-auto max-w-[620px] text-sm leading-relaxed text-muted-foreground">
          <Trans
            i18nKey="chat.projectWelcomeTitle"
            values={{ project: projectName }}
            components={{
              project: (
                <span className="font-medium text-foreground underline decoration-border decoration-dotted underline-offset-4" />
              ),
            }}
          />
        </p>
        <div className="mx-auto mt-7 grid w-full max-w-[1040px] grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-4">
          {PROJECT_WELCOME_SUGGESTIONS.map((suggestion) => {
            const Icon = suggestion.icon
            const title = t(`chat.projectWelcomeSuggestions.${suggestion.key}.title`)
            const prompt = t(`chat.projectWelcomeSuggestions.${suggestion.key}.prompt`, {
              project: projectName,
            })
            return (
              <button
                key={suggestion.key}
                type="button"
                onClick={() => onProjectSuggestion?.(prompt)}
                disabled={!onProjectSuggestion}
                className="group flex min-h-28 cursor-pointer flex-col items-start justify-between rounded-2xl border border-border/70 bg-background/55 p-4 text-left shadow-sm transition-all hover:-translate-y-0.5 hover:border-border hover:bg-muted/30 hover:shadow-md disabled:cursor-default disabled:hover:translate-y-0 disabled:hover:border-border/70 disabled:hover:bg-background/55 disabled:hover:shadow-sm"
              >
                <Icon
                  className={cn(
                    "h-5 w-5 shrink-0 transition-transform group-hover:-translate-y-px",
                    suggestion.iconClassName,
                  )}
                />
                <span className="mt-5 text-sm font-medium leading-snug text-foreground">{title}</span>
              </button>
            )
          })}
        </div>
      </div>
    )
  }

  return (
    <div className="px-4 text-center">
      <img
        src={alphaLogoUrl}
        alt=""
        className="mx-auto mb-5 h-[72px] w-[72px] object-contain opacity-95"
        draggable={false}
      />
      <p className="text-sm text-muted-foreground">{t("chat.howCanIHelp")}</p>
    </div>
  )
}
