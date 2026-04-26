import { useState } from "react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { SUPPORTED_LANGUAGES, isFollowingSystem, setFollowSystemLanguage, setLanguage } from "@/i18n/i18n"
import { Button } from "@/components/ui/button"
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
    <div className="flex-1 overflow-y-auto p-6">
      <h2 className="text-lg font-semibold text-foreground mb-1">{t("settings.language")}</h2>
      <p className="text-xs text-muted-foreground mb-5">{t("settings.languageDesc")}</p>

      <div className="space-y-0.5">
        {/* Follow System option */}
        <Button
          variant="ghost"
          className={cn(
            "h-auto w-full justify-start gap-3 rounded-lg px-3 py-2.5 text-sm",
            followSystem
              ? "bg-primary/10 text-primary font-medium hover:bg-primary/10 hover:text-primary"
              : "text-foreground hover:bg-secondary/60",
          )}
          onClick={handleFollowSystem}
        >
          <span className={cn("shrink-0", followSystem ? "text-primary" : "text-muted-foreground")}>
            <Monitor className="h-4 w-4" />
          </span>
          <span className="flex-1 text-left">{t("language.system")}</span>
          {followSystem && <Check className="h-4 w-4 text-primary shrink-0" />}
        </Button>

        {/* Divider */}
        <div className="border-t border-border/50 my-1.5" />

        {SUPPORTED_LANGUAGES.map((lang) => (
          <Button
            key={lang.code}
            variant="ghost"
            className={cn(
              "h-auto w-full justify-start gap-3 rounded-lg px-3 py-2.5 text-sm",
              isCurrentLang(lang.code)
                ? "bg-primary/10 text-primary font-medium hover:bg-primary/10 hover:text-primary"
                : "text-foreground hover:bg-secondary/60",
            )}
            onClick={() => handleSelectLanguage(lang.code)}
          >
            <span className="text-xs font-bold w-6 text-center opacity-60">{lang.shortLabel}</span>
            <span className="flex-1 text-left">{lang.label}</span>
            {isCurrentLang(lang.code) && <Check className="h-4 w-4 text-primary shrink-0" />}
          </Button>
        ))}
      </div>
    </div>
  )
}
