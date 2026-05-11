import { useTranslation } from "react-i18next"

import WebSearchPanel from "@/components/settings/WebSearchPanel"

interface SearchProviderStepProps {
  onSaved: () => void
}

/**
 * Optional web search setup.
 *
 * The Settings panel owns the provider ordering, credential fields, and
 * validation rules. Onboarding embeds the same panel so search provider
 * configuration stays a single UI contract.
 */
export function SearchProviderStep({ onSaved }: SearchProviderStepProps) {
  const { t } = useTranslation()

  return (
    <div className="px-4 py-6">
      <div className="max-w-3xl mx-auto mb-4 text-center space-y-1">
        <h2 className="text-xl font-semibold">{t("onboarding.searchProvider.title")}</h2>
        <p className="text-sm text-muted-foreground">
          {t("onboarding.searchProvider.subtitle")}
        </p>
      </div>
      <div className="max-w-3xl mx-auto">
        <WebSearchPanel
          embedded
          showAdvanced={false}
          saveLabel={t("onboarding.searchProvider.saveAndContinue")}
          onSaved={onSaved}
        />
      </div>
    </div>
  )
}
