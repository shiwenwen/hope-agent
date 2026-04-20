import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Globe, Sparkles } from "lucide-react"

import { setLanguage, setFollowSystemLanguage, SUPPORTED_LANGUAGES } from "@/i18n/i18n"

interface WelcomeStepProps {
  /** Current language as stored in `config.language` ("auto" / "zh-CN" / ...). */
  initialLanguage: string
  onLanguageChange: (lang: string) => void
}

/**
 * Step 1 — welcome + language picker.
 *
 * Writing is immediate: switching language fires `setLanguage` /
 * `setFollowSystemLanguage` so the wizard UI itself re-renders in the
 * target locale. We avoid persisting through the onboarding draft — the
 * existing i18n plumbing already writes to `config.json`, and that's the
 * same path Step 1's "apply" would hit anyway.
 */
export function WelcomeStep({ initialLanguage, onLanguageChange }: WelcomeStepProps) {
  const { t, i18n } = useTranslation()
  const [value, setValue] = useState(initialLanguage || "auto")

  useEffect(() => {
    setValue(initialLanguage || "auto")
  }, [initialLanguage])

  async function handleSelect(next: string) {
    setValue(next)
    onLanguageChange(next)
    if (next === "auto") {
      await setFollowSystemLanguage()
    } else {
      await setLanguage(next)
    }
  }

  return (
    <div className="px-6 py-8 space-y-6">
      <div className="flex flex-col items-center text-center gap-3">
        <div className="flex items-center justify-center h-16 w-16 rounded-2xl bg-primary/10 text-primary">
          <Sparkles className="h-9 w-9" />
        </div>
        <h1 className="text-2xl font-semibold">{t("onboarding.welcome.title")}</h1>
        <p className="max-w-md text-sm text-muted-foreground leading-relaxed">
          {t("onboarding.welcome.subtitle")}
        </p>
      </div>

      <div className="space-y-2 max-w-sm mx-auto">
        <label className="flex items-center gap-1.5 text-sm font-medium">
          <Globe className="h-4 w-4" /> {t("onboarding.welcome.languageLabel")}
        </label>
        <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
          <button
            type="button"
            onClick={() => handleSelect("auto")}
            className={`rounded-md border px-3 py-2 text-sm transition-colors ${
              value === "auto"
                ? "border-primary bg-primary/10 text-primary"
                : "border-border hover:border-foreground/30"
            }`}
          >
            {t("onboarding.welcome.autoLanguage")}
          </button>
          {SUPPORTED_LANGUAGES.map((lang) => (
            <button
              key={lang.code}
              type="button"
              onClick={() => handleSelect(lang.code)}
              className={`rounded-md border px-3 py-2 text-sm transition-colors ${
                value === lang.code
                  ? "border-primary bg-primary/10 text-primary"
                  : "border-border hover:border-foreground/30"
              }`}
            >
              {lang.label}
            </button>
          ))}
        </div>
        <p className="text-xs text-muted-foreground">
          {t("onboarding.welcome.languageHint", { current: i18n.language })}
        </p>
      </div>
    </div>
  )
}
