import { useState } from "react"
import { useTranslation } from "react-i18next"
import { SUPPORTED_LANGUAGES, isFollowingSystem, setFollowSystemLanguage, setLanguage } from "@/i18n/i18n"
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
  SelectSeparator,
} from "@/components/ui/select"

export default function LanguageSection() {
  const { t, i18n } = useTranslation()
  const [followSystem, setFollowSystem] = useState(isFollowingSystem)

  const handleFollowSystem = () => {
    setFollowSystemLanguage()
    setFollowSystem(true)
  }

  const handleSelectLanguage = (code: string) => {
    setLanguage(code)
    setFollowSystem(false)
  }

  return (
    <div>
      <h3 className="text-sm font-semibold text-foreground mb-1">{t("settings.language")}</h3>
      <p className="text-xs text-muted-foreground mb-3">{t("settings.languageDesc")}</p>
      <Select
        value={
          followSystem
            ? "system"
            : (SUPPORTED_LANGUAGES.find(
                (l) => i18n.language === l.code || i18n.language.startsWith(l.code + "-"),
              )?.code ?? "system")
        }
        onValueChange={(val) => {
          if (val === "system") handleFollowSystem()
          else handleSelectLanguage(val)
        }}
      >
        <SelectTrigger className="w-full max-w-xs">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="system">{t("language.system")}</SelectItem>
          <SelectSeparator />
          {SUPPORTED_LANGUAGES.map((lang) => (
            <SelectItem key={lang.code} value={lang.code}>
              {lang.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  )
}
