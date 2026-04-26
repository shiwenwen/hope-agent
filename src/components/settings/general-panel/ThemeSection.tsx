import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { useTheme, type ThemeMode } from "@/hooks/useTheme"
import { Button } from "@/components/ui/button"
import { Monitor, Sun, Moon, Check } from "lucide-react"

const THEME_OPTIONS: {
  mode: ThemeMode
  icon: React.ReactNode
  labelKey: string
  descKey: string
}[] = [
  { mode: "auto", icon: <Monitor className="h-5 w-5" />, labelKey: "theme.auto", descKey: "theme.autoDesc" },
  { mode: "light", icon: <Sun className="h-5 w-5" />, labelKey: "theme.light", descKey: "theme.lightDesc" },
  { mode: "dark", icon: <Moon className="h-5 w-5" />, labelKey: "theme.dark", descKey: "theme.darkDesc" },
]

export default function ThemeSection() {
  const { t } = useTranslation()
  const { theme, setTheme } = useTheme()

  return (
    <div>
      <h3 className="text-sm font-semibold text-foreground mb-1">{t("settings.appearance")}</h3>
      <p className="text-xs text-muted-foreground mb-3">{t("settings.appearanceDesc")}</p>
      <div className="space-y-1">
        {THEME_OPTIONS.map((opt) => (
          <Button
            key={opt.mode}
            variant="ghost"
            className={cn(
              "h-auto w-full justify-start gap-3 rounded-lg px-3 py-3 text-sm",
              theme === opt.mode
                ? "bg-primary/10 text-primary font-medium hover:bg-primary/10 hover:text-primary"
                : "text-foreground hover:bg-secondary/60",
            )}
            onClick={() => setTheme(opt.mode)}
          >
            <span className={cn("shrink-0", theme === opt.mode ? "text-primary" : "text-muted-foreground")}>
              {opt.icon}
            </span>
            <div className="flex-1 text-left">
              <div>{t(opt.labelKey)}</div>
              <div className="text-xs text-muted-foreground font-normal">{t(opt.descKey)}</div>
            </div>
            {theme === opt.mode && <Check className="h-4 w-4 text-primary shrink-0" />}
          </Button>
        ))}
      </div>
    </div>
  )
}
