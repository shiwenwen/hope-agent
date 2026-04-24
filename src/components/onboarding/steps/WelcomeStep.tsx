import { useTranslation } from "react-i18next"
import { Check, Globe, Monitor, Moon, Sun } from "lucide-react"

import logoUrl from "@/assets/logo.png"
import { setLanguage, setFollowSystemLanguage, SUPPORTED_LANGUAGES } from "@/i18n/i18n"
import { cn } from "@/lib/utils"
import { setThemePreference, type ThemeMode } from "@/hooks/useTheme"

interface WelcomeStepProps {
  /** Current language as stored in `config.language` ("auto" / "zh-CN" / ...). */
  initialLanguage: string
  /** Current theme as stored in `config.theme` ("auto" / "light" / "dark"). */
  initialTheme: ThemeMode
  onLanguageChange: (lang: string) => void
  onThemeChange: (theme: ThemeMode) => void
}

const THEME_OPTIONS: Array<{
  mode: ThemeMode
  icon: typeof Monitor
  labelKey: string
  descKey: string
}> = [
  { mode: "auto", icon: Monitor, labelKey: "theme.auto", descKey: "theme.autoDesc" },
  { mode: "light", icon: Sun, labelKey: "theme.light", descKey: "theme.lightDesc" },
  { mode: "dark", icon: Moon, labelKey: "theme.dark", descKey: "theme.darkDesc" },
]

/**
 * Step 1 — welcome + language picker.
 *
 * Writing is immediate: switching language fires `setLanguage` /
 * `setFollowSystemLanguage` so the wizard UI itself re-renders in the
 * target locale. We avoid persisting through the onboarding draft — the
 * existing i18n plumbing already writes to `config.json`, and that's the
 * same path Step 1's "apply" would hit anyway.
 */
export function WelcomeStep({
  initialLanguage,
  initialTheme,
  onLanguageChange,
  onThemeChange,
}: WelcomeStepProps) {
  const { t, i18n } = useTranslation()
  const value = initialLanguage || "auto"
  const theme = initialTheme || "auto"

  async function handleSelect(next: string) {
    onLanguageChange(next)
    if (next === "auto") {
      await setFollowSystemLanguage()
    } else {
      await setLanguage(next)
    }
  }

  function handleThemeSelect(next: ThemeMode) {
    onThemeChange(next)
    setThemePreference(next)
  }

  return (
    <div className="px-8 py-10 space-y-8">
      <div className="flex flex-col items-center text-center gap-4">
        <img
          src={logoUrl}
          alt="Hope Agent"
          className="h-20 w-20 rounded-2xl shadow-md"
          draggable={false}
        />
        <h1 className="text-3xl font-semibold tracking-tight">
          {t("onboarding.welcome.title")}
        </h1>
        <p className="max-w-lg text-base text-muted-foreground leading-relaxed whitespace-pre-line">
          {t("onboarding.welcome.subtitle")}
        </p>
      </div>

      <div className="space-y-3">
        <label className="flex items-center gap-2 text-sm font-medium">
          <Globe className="h-4 w-4" /> {t("onboarding.welcome.languageLabel")}
        </label>
        <div className="grid grid-cols-3 gap-2 sm:grid-cols-4">
          <button
            type="button"
            onClick={() => handleSelect("auto")}
            className={`rounded-lg border px-3 py-2.5 text-sm transition-colors ${
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
              className={`rounded-lg border px-3 py-2.5 text-sm transition-colors ${
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

      <div className="space-y-3">
        <label className="flex items-center gap-2 text-sm font-medium">
          <Sun className="h-4 w-4" /> {t("settings.appearance")}
        </label>
        <div className="grid gap-2 sm:grid-cols-3">
          {THEME_OPTIONS.map((opt) => {
            const Icon = opt.icon
            const active = theme === opt.mode
            return (
              <button
                key={opt.mode}
                type="button"
                onClick={() => handleThemeSelect(opt.mode)}
                className={cn(
                  "flex items-center gap-3 rounded-lg border px-3 py-3 text-left text-sm transition-colors",
                  active
                    ? "border-primary bg-primary/10 text-primary"
                    : "border-border hover:border-foreground/30",
                )}
              >
                <Icon className={cn("h-4 w-4 shrink-0", active ? "text-primary" : "text-muted-foreground")} />
                <div className="min-w-0 flex-1">
                  <div className="font-medium">{t(opt.labelKey)}</div>
                  <div className="text-xs text-muted-foreground">{t(opt.descKey)}</div>
                </div>
                {active && <Check className="h-4 w-4 shrink-0 text-primary" />}
              </button>
            )
          })}
        </div>
      </div>
    </div>
  )
}
