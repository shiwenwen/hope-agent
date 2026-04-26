import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"

import LocalLlmAssistantCard from "@/components/settings/local-llm/LocalLlmAssistantCard"
import { hasLocalOllamaProvider } from "@/components/settings/local-llm/provider-detection"
import ProviderSetup from "@/components/settings/ProviderSetup"
import type { ProviderConfig } from "@/components/settings/provider-setup"
import { logger } from "@/lib/logger"
import { getTransport } from "@/lib/transport-provider"

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
  const [providers, setProviders] = useState<ProviderConfig[] | null>(null)

  useEffect(() => {
    let cancelled = false

    async function loadProviders() {
      try {
        const list = await getTransport().call<ProviderConfig[]>("get_providers")
        if (!cancelled) setProviders(list)
      } catch (e) {
        logger.warn("onboarding", "ProviderStep::loadProviders", "Failed to load providers", e)
        if (!cancelled) setProviders([])
      }
    }

    void loadProviders()

    return () => {
      cancelled = true
    }
  }, [])

  // Codex OAuth bypasses ProviderSetup's onComplete, so advance manually.
  async function handleCodexAuthInOnboarding() {
    await onCodexAuth()
    onProviderSaved()
  }

  const showLocalLlmAssistant = providers !== null && !hasLocalOllamaProvider(providers)

  return (
    <div className="px-4 py-6">
      <div className="max-w-3xl mx-auto mb-4 text-center space-y-1">
        <h2 className="text-xl font-semibold">{t("onboarding.provider.title")}</h2>
        <p className="text-sm text-muted-foreground">{t("onboarding.provider.subtitle")}</p>
      </div>
      {showLocalLlmAssistant && (
        <div className="max-w-3xl mx-auto mb-4">
          <LocalLlmAssistantCard onProviderInstalled={onProviderSaved} />
        </div>
      )}
      <div className="max-w-3xl mx-auto">
        <ProviderSetup
          onComplete={onProviderSaved}
          onCodexAuth={handleCodexAuthInOnboarding}
          hideRemoteConnect
          embedded
        />
      </div>
    </div>
  )
}
