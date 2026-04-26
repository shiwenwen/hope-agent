import { useTranslation } from "react-i18next"

import ProviderSetup from "@/components/settings/ProviderSetup"

interface ProviderStepProps {
  onProviderSaved: () => void
  onCodexAuth: () => Promise<void>
}

/**
 * Step 2 — pick or configure the first model provider.
 *
 * Wraps the existing `ProviderSetup` component so we don't duplicate its
 * template grid, custom wizard, and Codex OAuth flow. On successful save
 * it calls `onProviderSaved` which routes through the onboarding
 * state-machine's `goNext` — matching the behaviour of clicking the real
 * Next button on other steps.
 */
export function ProviderStep({ onProviderSaved, onCodexAuth }: ProviderStepProps) {
  const { t } = useTranslation()

  // Codex OAuth bypasses ProviderSetup's onComplete, so advance manually.
  async function handleCodexAuthInOnboarding() {
    await onCodexAuth()
    onProviderSaved()
  }

  return (
    <div className="px-4 py-6">
      <div className="max-w-3xl mx-auto mb-4 text-center space-y-1">
        <h2 className="text-xl font-semibold">{t("onboarding.provider.title")}</h2>
        <p className="text-sm text-muted-foreground">{t("onboarding.provider.subtitle")}</p>
      </div>
      <div className="max-w-3xl mx-auto">
        <ProviderSetup
          onComplete={onProviderSaved}
          onCodexAuth={handleCodexAuthInOnboarding}
          hideRemoteConnect
          showLocalLlmAssistant
          onLocalLlmInstalled={onProviderSaved}
          embedded
        />
      </div>
    </div>
  )
}
