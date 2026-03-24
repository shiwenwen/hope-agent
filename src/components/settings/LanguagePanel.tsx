import { useState } from "react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { SUPPORTED_LANGUAGES, isFollowingSystem, setFollowSystemLanguage, setLanguage } from "@/i18n/i18n"
import { Monitor, Check } from "lucide-react"

export default function LanguagePanel() {
  const { t, i18n } = useTranslation()
  const [followSystem, setFollowSystem] = useState(isFollowingSystem)

  const isCurrentLang = (code: string) => {
    if (followSystem) return false
    return i18n.language === code || (i18n.language.startsWith(code + "-") && code !== "zh")
  }

  const handleFollowSystem = () => {
    setFollowSystemLanguage()
    setFollowSystem(true)
  }

  const handleSelectLanguage = (code: string) => {
    setLanguage(code)
    setFollowSystem(false)
  }

  return (
    <div className="flex-1 overflow-y-auto p-6 max-w-4xl">
      <h2 className="text-lg font-semibold text-foreground mb-1">{t("settings.language")}</h2>
      <p className="text-xs text-muted-foreground mb-5">{t("settings.languageDesc")}</p>

      <div className="space-y-0.5">
        {/* Follow System option */}
        <button
          className={cn(
            "flex items-center gap-3 w-full px-3 py-2.5 rounded-lg text-sm transition-colors",
            followSystem
              ? "bg-primary/10 text-primary font-medium"
              : "text-foreground hover:bg-secondary/60",
          )}
          onClick={handleFollowSystem}
        >
          <span className={cn("shrink-0", followSystem ? "text-primary" : "text-muted-foreground")}>
            <Monitor className="h-4 w-4" />
          </span>
          <span className="flex-1 text-left">{t("language.system")}</span>
          {followSystem && <Check className="h-4 w-4 text-primary shrink-0" />}
        </button>

        {/* Divider */}
        <div className="border-t border-border/50 my-1.5" />

        {SUPPORTED_LANGUAGES.map((lang) => (
          <button
            key={lang.code}
            className={cn(
              "flex items-center gap-3 w-full px-3 py-2.5 rounded-lg text-sm transition-colors",
              isCurrentLang(lang.code)
                ? "bg-primary/10 text-primary font-medium"
                : "text-foreground hover:bg-secondary/60",
            )}
            onClick={() => handleSelectLanguage(lang.code)}
          >
            <span className="text-xs font-bold w-6 text-center opacity-60">{lang.shortLabel}</span>
            <span className="flex-1 text-left">{lang.label}</span>
            {isCurrentLang(lang.code) && <Check className="h-4 w-4 text-primary shrink-0" />}
          </button>
        ))}
      </div>
    </div>
  )
}
