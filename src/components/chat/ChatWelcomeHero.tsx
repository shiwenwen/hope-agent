import { Ghost } from "lucide-react"
import { useTranslation } from "react-i18next"

import alphaLogoUrl from "@/assets/alpha-logo.png"

/**
 * The empty-session greeting (logo + slogan, or the incognito notice). Shared by
 * {@link MessageList} (plain centered empty state) and `ChatScreen`'s hero
 * composer, where it sits directly above the centered input as a single
 * vertically-centered unit — so the greeting and the composer can never overlap
 * regardless of width/height (the two used to center independently and collided
 * when the pane was squeezed).
 */
export function ChatWelcomeHero({ incognito = false }: { incognito?: boolean }) {
  const { t } = useTranslation()

  if (incognito) {
    return (
      <div className="max-w-[360px] px-4 text-center text-muted-foreground">
        <Ghost className="mx-auto mb-3 h-6 w-6" />
        <div className="text-sm font-semibold text-foreground/70">
          {t("chat.incognitoEmptyTitle")}
        </div>
        <p className="mt-2 text-sm leading-relaxed">{t("chat.incognitoEmptyBody")}</p>
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
