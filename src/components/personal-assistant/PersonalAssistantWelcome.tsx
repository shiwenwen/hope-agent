import { useTranslation } from "react-i18next"
import type { AssistantProfile } from "./usePersonalAssistant"

export function PersonalAssistantWelcome({ profile }: { profile: AssistantProfile | null }) {
  const { t } = useTranslation()
  return (
    <div className="mx-auto max-w-lg px-4 text-center">
      <h1 className="text-2xl font-medium">
        {t("personalAssistant.welcomeTitle", { name: profile?.name || "Hope" })}
      </h1>
    </div>
  )
}
