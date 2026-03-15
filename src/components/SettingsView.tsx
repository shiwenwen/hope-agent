import { useState } from "react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import {
  ArrowLeft,
  Check,
  Globe,
  Info,
  Monitor,
  Moon,
  Palette,
  Server,
  Sun,
} from "lucide-react"
import { SUPPORTED_LANGUAGES, isFollowingSystem, setFollowSystemLanguage } from "@/i18n/i18n"
import { useTheme, type ThemeMode } from "@/hooks/useTheme"
import ProviderSettings from "@/components/ProviderSettings"
import type { ProviderConfig } from "@/components/ProviderSettings"
import ProviderSetup from "@/components/ProviderSetup"
import ProviderEditPage from "@/components/ProviderEditPage"

type SettingsSection = "providers" | "appearance" | "language" | "about"

interface SettingsSectionItem {
  id: SettingsSection
  icon: React.ReactNode
  labelKey: string
}

const SECTIONS: SettingsSectionItem[] = [
  {
    id: "providers",
    icon: <Server className="h-4 w-4" />,
    labelKey: "settings.providers",
  },
  {
    id: "appearance",
    icon: <Palette className="h-4 w-4" />,
    labelKey: "settings.appearance",
  },
  {
    id: "language",
    icon: <Globe className="h-4 w-4" />,
    labelKey: "settings.language",
  },
  {
    id: "about",
    icon: <Info className="h-4 w-4" />,
    labelKey: "settings.about",
  },
]

// ── Appearance Settings Panel ─────────────────────────────────────

const THEME_OPTIONS: { mode: ThemeMode; icon: React.ReactNode; labelKey: string; descKey: string }[] = [
  { mode: "auto", icon: <Monitor className="h-5 w-5" />, labelKey: "theme.auto", descKey: "theme.autoDesc" },
  { mode: "light", icon: <Sun className="h-5 w-5" />, labelKey: "theme.light", descKey: "theme.lightDesc" },
  { mode: "dark", icon: <Moon className="h-5 w-5" />, labelKey: "theme.dark", descKey: "theme.darkDesc" },
]

function AppearancePanel() {
  const { t } = useTranslation()
  const { theme, setTheme } = useTheme()

  return (
    <div className="p-6 max-w-xl">
      <h2 className="text-lg font-semibold text-foreground mb-1">
        {t("settings.appearance")}
      </h2>
      <p className="text-xs text-muted-foreground mb-5">
        {t("settings.appearanceDesc")}
      </p>

      <div className="space-y-1">
        {THEME_OPTIONS.map((opt) => (
          <button
            key={opt.mode}
            className={cn(
              "flex items-center gap-3 w-full px-3 py-3 rounded-lg text-sm transition-colors",
              theme === opt.mode
                ? "bg-primary/10 text-primary font-medium"
                : "text-foreground hover:bg-secondary/60"
            )}
            onClick={() => setTheme(opt.mode)}
          >
            <span
              className={cn(
                "shrink-0",
                theme === opt.mode ? "text-primary" : "text-muted-foreground"
              )}
            >
              {opt.icon}
            </span>
            <div className="flex-1 text-left">
              <div>{t(opt.labelKey)}</div>
              <div className="text-xs text-muted-foreground font-normal">
                {t(opt.descKey)}
              </div>
            </div>
            {theme === opt.mode && (
              <Check className="h-4 w-4 text-primary shrink-0" />
            )}
          </button>
        ))}
      </div>
    </div>
  )
}

// ── Language Settings Panel ───────────────────────────────────────

function LanguagePanel() {
  const { t, i18n } = useTranslation()
  const [followSystem, setFollowSystem] = useState(isFollowingSystem)

  const isCurrentLang = (code: string) => {
    if (followSystem) return false
    return (
      i18n.language === code ||
      (i18n.language.startsWith(code + "-") && code !== "zh")
    )
  }

  const handleFollowSystem = () => {
    setFollowSystemLanguage()
    setFollowSystem(true)
  }

  const handleSelectLanguage = (code: string) => {
    i18n.changeLanguage(code)
    setFollowSystem(false)
  }

  return (
    <div className="p-6 max-w-xl">
      <h2 className="text-lg font-semibold text-foreground mb-1">
        {t("settings.language")}
      </h2>
      <p className="text-xs text-muted-foreground mb-5">
        {t("settings.languageDesc")}
      </p>

      <div className="space-y-0.5">
        {/* Follow System option */}
        <button
          className={cn(
            "flex items-center gap-3 w-full px-3 py-2.5 rounded-lg text-sm transition-colors",
            followSystem
              ? "bg-primary/10 text-primary font-medium"
              : "text-foreground hover:bg-secondary/60"
          )}
          onClick={handleFollowSystem}
        >
          <span className={cn("shrink-0", followSystem ? "text-primary" : "text-muted-foreground")}>
            <Monitor className="h-4 w-4" />
          </span>
          <span className="flex-1 text-left">{t("language.system")}</span>
          {followSystem && (
            <Check className="h-4 w-4 text-primary shrink-0" />
          )}
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
                : "text-foreground hover:bg-secondary/60"
            )}
            onClick={() => handleSelectLanguage(lang.code)}
          >
            <span className="text-xs font-bold w-6 text-center opacity-60">
              {lang.shortLabel}
            </span>
            <span className="flex-1 text-left">{lang.label}</span>
            {isCurrentLang(lang.code) && (
              <Check className="h-4 w-4 text-primary shrink-0" />
            )}
          </button>
        ))}
      </div>
    </div>
  )
}

// ── About Panel ───────────────────────────────────────────────────

function AboutPanel() {
  const { t } = useTranslation()

  return (
    <div className="p-6 max-w-xl">
      <div className="flex flex-col items-center text-center py-8">
        {/* App Icon */}
        <div className="w-20 h-20 rounded-2xl bg-gradient-to-br from-primary/20 via-primary/10 to-transparent border border-border/50 flex items-center justify-center mb-5 shadow-lg">
          <span className="text-3xl font-bold text-primary">OC</span>
        </div>

        <h2 className="text-xl font-bold text-foreground mb-1">
          OpenComputer
        </h2>
        <p className="text-xs text-muted-foreground mb-4">
          {t("about.version")} 0.1.0
        </p>

        <p className="text-sm text-muted-foreground leading-relaxed max-w-sm mb-6">
          {t("about.description")}
        </p>

        <div className="flex items-center gap-4">
          <a
            href="https://github.com"
            target="_blank"
            rel="noreferrer"
            className="text-xs text-muted-foreground hover:text-foreground transition-colors underline underline-offset-2"
          >
            {t("about.github")}
          </a>
        </div>
      </div>

      {/* Tech Stack */}
      <div className="border-t border-border pt-5 mt-2">
        <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-3">
          {t("about.techStack")}
        </h3>
        <div className="grid grid-cols-2 gap-2 text-xs">
          {[
            ["Frontend", "React 19 + TypeScript"],
            ["Backend", "Rust + Tauri 2"],
            ["Styling", "Tailwind CSS v4"],
            ["UI", "shadcn/ui (Radix)"],
          ].map(([label, value]) => (
            <div
              key={label}
              className="flex flex-col gap-0.5 bg-secondary/40 rounded-lg px-3 py-2 border border-border/30"
            >
              <span className="text-muted-foreground">{label}</span>
              <span className="text-foreground font-medium">{value}</span>
            </div>
          ))}
        </div>
      </div>
    </div>
  )
}

// ── Main SettingsView Component ───────────────────────────────────

export default function SettingsView({
  onBack,
  onCodexAuth,
  onCodexReauth,
}: {
  onBack: () => void
  onCodexAuth: () => Promise<void>
  onCodexReauth?: () => void
}) {
  const { t } = useTranslation()
  const [activeSection, setActiveSection] =
    useState<SettingsSection>("providers")
  const [addingProvider, setAddingProvider] = useState(false)
  const [editingProvider, setEditingProvider] = useState<ProviderConfig | null>(null)

  return (
    <div className="flex flex-1 h-full overflow-hidden bg-background">
      {/* Left Sidebar — Settings Navigation */}
      <div className="w-[220px] shrink-0 border-r border-border bg-secondary/20 flex flex-col">
        {/* Header with back button + drag region */}
        <div className="h-10 flex items-end px-4 gap-2 shrink-0" data-tauri-drag-region>
          <button
            onClick={onBack}
            className="flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground transition-colors pb-1.5"
          >
            <ArrowLeft className="h-4 w-4" />
            <span className="text-sm font-semibold text-foreground">
              {t("settings.title")}
            </span>
          </button>
        </div>

        {/* Navigation Items */}
        <div className="flex-1 overflow-y-auto p-2 space-y-0.5">
          {SECTIONS.map((section) => (
            <button
              key={section.id}
              className={cn(
                "flex items-center gap-2.5 w-full px-3 py-2 rounded-lg text-sm transition-all duration-150",
                activeSection === section.id
                  ? "bg-secondary text-foreground font-medium shadow-sm"
                  : "text-muted-foreground hover:bg-secondary/60 hover:text-foreground"
              )}
              onClick={() => setActiveSection(section.id)}
            >
              <span
                className={cn(
                  "shrink-0",
                  activeSection === section.id
                    ? "text-primary"
                    : "text-muted-foreground"
                )}
              >
                {section.icon}
              </span>
              {t(section.labelKey)}
            </button>
          ))}
        </div>
      </div>

      {/* Right Content Panel */}
      <div className="flex-1 flex flex-col min-w-0 overflow-hidden">
        {/* Content Header + drag region */}
        <div className="h-10 flex items-end px-6 shrink-0" data-tauri-drag-region>
          <span className="text-sm font-semibold text-foreground pb-1.5">
            {t(
              SECTIONS.find((s) => s.id === activeSection)?.labelKey ??
                "settings.title"
            )}
          </span>
        </div>

        {/* Content Area */}
        <div className="flex-1 overflow-hidden">
          {activeSection === "providers" && (
            addingProvider ? (
              <ProviderSetup
                onComplete={() => setAddingProvider(false)}
                onCodexAuth={onCodexAuth}
                onCancel={() => setAddingProvider(false)}
              />
            ) : editingProvider ? (
              <ProviderEditPage
                provider={editingProvider}
                onSave={() => setEditingProvider(null)}
                onCancel={() => setEditingProvider(null)}
                onCodexReauth={onCodexReauth}
              />
            ) : (
              <ProviderSettings
                onAddProvider={() => setAddingProvider(true)}
                onEditProvider={(p) => setEditingProvider(p)}
                onCodexReauth={onCodexReauth}
              />
            )
          )}
          {activeSection === "appearance" && <AppearancePanel />}
          {activeSection === "language" && <LanguagePanel />}
          {activeSection === "about" && <AboutPanel />}
        </div>
      </div>
    </div>
  )
}
