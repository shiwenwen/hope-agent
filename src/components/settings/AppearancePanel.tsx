import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { useTheme, type ThemeMode } from "@/hooks/useTheme"
import { Monitor, Sun, Moon, Check } from "lucide-react"

const THEME_OPTIONS: {
  mode: ThemeMode
  icon: React.ReactNode
  labelKey: string
  descKey: string
}[] = [
  {
    mode: "auto",
    icon: <Monitor className="h-5 w-5" />,
    labelKey: "theme.auto",
    descKey: "theme.autoDesc",
  },
  {
    mode: "light",
    icon: <Sun className="h-5 w-5" />,
    labelKey: "theme.light",
    descKey: "theme.lightDesc",
  },
  {
    mode: "dark",
    icon: <Moon className="h-5 w-5" />,
    labelKey: "theme.dark",
    descKey: "theme.darkDesc",
  },
]

export default function AppearancePanel() {
  const { t } = useTranslation()
  const { theme, setTheme } = useTheme()

  return (
    <div className="flex-1 overflow-y-auto p-6 max-w-4xl">
      <h2 className="text-lg font-semibold text-foreground mb-1">{t("settings.appearance")}</h2>
      <p className="text-xs text-muted-foreground mb-5">{t("settings.appearanceDesc")}</p>

      <div className="space-y-1">
        {THEME_OPTIONS.map((opt) => (
          <button
            key={opt.mode}
            className={cn(
              "flex items-center gap-3 w-full px-3 py-3 rounded-lg text-sm transition-colors",
              theme === opt.mode
                ? "bg-primary/10 text-primary font-medium"
                : "text-foreground hover:bg-secondary/60",
            )}
            onClick={() => setTheme(opt.mode)}
          >
            <span
              className={cn(
                "shrink-0",
                theme === opt.mode ? "text-primary" : "text-muted-foreground",
              )}
            >
              {opt.icon}
            </span>
            <div className="flex-1 text-left">
              <div>{t(opt.labelKey)}</div>
              <div className="text-xs text-muted-foreground font-normal">{t(opt.descKey)}</div>
            </div>
            {theme === opt.mode && <Check className="h-4 w-4 text-primary shrink-0" />}
          </button>
        ))}
      </div>
    </div>
  )
}
