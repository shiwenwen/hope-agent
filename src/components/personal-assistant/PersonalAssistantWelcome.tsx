import { useTranslation } from "react-i18next"
import { PersonalAssistantAvatar } from "./PersonalAssistantAvatar"
import type { AssistantProfile } from "./usePersonalAssistant"

export function PersonalAssistantWelcome({ profile }: { profile: AssistantProfile | null }) {
  const { t } = useTranslation()
  return (
    <div className="mx-auto max-w-lg px-4 text-center">
      <PersonalAssistantAvatar avatar={profile?.avatar} className="mb-5 h-16 w-16" />
      <h1 className="text-2xl font-medium">
        {t("personalAssistant.welcomeTitle", { name: profile?.name || "Hope" })}
      </h1>
      <p className="mt-3 text-sm leading-relaxed text-muted-foreground">
        {t("personalAssistant.welcomeBody")}
      </p>
    </div>
  )
}
