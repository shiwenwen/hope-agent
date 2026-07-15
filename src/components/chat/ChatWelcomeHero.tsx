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
  context = "chat",
  projectName,
  onProjectSuggestion,
}: {
  incognito?: boolean
  context?: "chat" | "knowledge"
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
        <div className="mx-auto mt-5 flex w-full max-w-[900px] flex-wrap justify-center gap-2">
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
                className="group inline-flex h-9 cursor-pointer items-center gap-2 rounded-full border border-border/70 bg-background/55 px-3.5 text-left text-xs font-medium text-foreground shadow-sm transition-colors hover:border-border hover:bg-muted/40 disabled:cursor-default disabled:opacity-60 disabled:hover:border-border/70 disabled:hover:bg-background/55"
              >
                <Icon
                  className={cn(
                    "h-4 w-4 shrink-0 transition-transform group-hover:scale-105",
                    suggestion.iconClassName,
                  )}
                />
                <span className="whitespace-nowrap">{title}</span>
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
      <p className="text-sm text-muted-foreground">
        {t(context === "knowledge" ? "knowledge.chatPanel.welcome" : "chat.howCanIHelp")}
      </p>
    </div>
  )
}
