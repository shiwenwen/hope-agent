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
      <div className="mx-auto max-w-[680px] px-4 text-center">
        <img
          src={alphaLogoUrl}
          alt=""
          className="mx-auto mb-5 h-[72px] w-[72px] object-contain opacity-35 grayscale"
          draggable={false}
        />
        <p className="mx-auto max-w-[620px] text-sm leading-relaxed text-muted-foreground">
          {t("chat.incognitoEmptyBody")}
        </p>
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
