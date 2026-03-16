import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import {
  ArrowLeft,
  Check,
  ChevronRight,
  Globe,
  Info,
  Monitor,
  Moon,
  Palette,
  Puzzle,
  Server,
  Sun,
  FolderOpen,
  File,
  Folder,
  ExternalLink,
  X,
  User,
} from "lucide-react"
import { SUPPORTED_LANGUAGES, isFollowingSystem, setFollowSystemLanguage } from "@/i18n/i18n"
import { useTheme, type ThemeMode } from "@/hooks/useTheme"
import { Switch } from "@/components/ui/switch"
import ProviderSettings from "@/components/ProviderSettings"
import type { ProviderConfig } from "@/components/ProviderSettings"
import ProviderSetup from "@/components/ProviderSetup"
import ProviderEditPage from "@/components/ProviderEditPage"

type SettingsSection = "providers" | "skills" | "profile" | "appearance" | "language" | "about"

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
    id: "skills",
    icon: <Puzzle className="h-4 w-4" />,
    labelKey: "settings.skills",
  },
  {
    id: "profile",
    icon: <User className="h-4 w-4" />,
    labelKey: "settings.profile",
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

// ── Skills Panel ──────────────────────────────────────────────────

interface SkillSummary {
  name: string
  description: string
  source: string
  base_dir: string
  enabled: boolean
}

interface SkillFileInfo {
  name: string
  size: number
  is_dir: boolean
}

interface SkillDetail {
  name: string
  description: string
  source: string
  file_path: string
  base_dir: string
  content: string
  enabled: boolean
  files: SkillFileInfo[]
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

function SkillsPanel() {
  const { t } = useTranslation()
  const [skills, setSkills] = useState<SkillSummary[]>([])
  const [extraDirs, setExtraDirs] = useState<string[]>([])
  const [selectedSkill, setSelectedSkill] = useState<SkillDetail | null>(null)
  const [loading, setLoading] = useState(true)

  async function reload() {
    try {
      const [list, dirs] = await Promise.all([
        invoke<SkillSummary[]>("get_skills"),
        invoke<string[]>("get_extra_skills_dirs"),
      ])
      setSkills(list)
      setExtraDirs(dirs)
    } catch (e) {
      console.error("Failed to load skills:", e)
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => { reload() }, [])

  async function handleOpenDir(path: string) {
    try {
      await invoke("open_directory", { path })
    } catch (e) {
      console.error("Failed to open directory:", e)
    }
  }

  async function handleAddDir() {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog")
      const selected = await open({ directory: true, multiple: false })
      if (selected) {
        await invoke("add_extra_skills_dir", { dir: selected })
        await reload()
      }
    } catch (e) {
      console.error("Failed to add skills directory:", e)
    }
  }

  async function handleRemoveDir(dir: string) {
    try {
      await invoke("remove_extra_skills_dir", { dir })
      await reload()
    } catch (e) {
      console.error("Failed to remove skills directory:", e)
    }
  }

  async function handleToggleSkill(name: string, enabled: boolean) {
    try {
      await invoke("toggle_skill", { name, enabled })
      // Update local state immediately
      setSkills((prev) =>
        prev.map((s) => (s.name === name ? { ...s, enabled } : s))
      )
      if (selectedSkill?.name === name) {
        setSelectedSkill((prev) => prev ? { ...prev, enabled } : prev)
      }
    } catch (e) {
      console.error("Failed to toggle skill:", e)
    }
  }

  async function handleSelectSkill(name: string) {
    try {
      const detail = await invoke<SkillDetail>("get_skill_detail", { name })
      setSelectedSkill(detail)
    } catch (e) {
      console.error("Failed to load skill detail:", e)
    }
  }

  // ── Skill Detail View ──────────────────────────────────────────
  if (selectedSkill) {
    return (
      <div className="flex-1 flex flex-col min-h-0 overflow-y-auto p-6">
        <div className="max-w-2xl">
          <button
            onClick={() => setSelectedSkill(null)}
            className="flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground transition-colors mb-4"
          >
            <ArrowLeft className="h-4 w-4" />
            <span>{t("settings.skills")}</span>
          </button>

          {/* Header */}
          <div className="mb-4">
            <div className="flex items-center gap-3">
              <h2 className="text-lg font-semibold text-foreground">{selectedSkill.name}</h2>
              <Switch
                checked={selectedSkill.enabled}
                onCheckedChange={(v) => handleToggleSkill(selectedSkill.name, v)}
              />
            </div>
            <p className="text-xs text-muted-foreground mt-1">{selectedSkill.description}</p>
            <div className="flex items-center gap-2 mt-2">
              <span className="text-[10px] px-1.5 py-0.5 rounded bg-secondary text-muted-foreground font-medium">
                {selectedSkill.source}
              </span>
              <button
                className="flex items-center gap-1 text-[10px] text-muted-foreground hover:text-foreground transition-colors"
                onClick={() => handleOpenDir(selectedSkill.base_dir)}
                title={selectedSkill.base_dir}
              >
                <ExternalLink className="h-3 w-3" />
                <span className="truncate max-w-[300px]">{selectedSkill.base_dir}</span>
              </button>
            </div>
          </div>

          {/* Files in skill directory */}
          {selectedSkill.files.length > 0 && (
            <div className="mb-4">
              <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">
                {t("settings.skillFiles")}
              </h3>
              <div className="rounded-lg border border-border overflow-hidden">
                {selectedSkill.files.map((file) => (
                  <div
                    key={file.name}
                    className="flex items-center gap-2 px-3 py-1.5 text-xs border-b border-border/50 last:border-b-0 bg-secondary/20"
                  >
                    {file.is_dir
                      ? <Folder className="h-3.5 w-3.5 text-primary/60 shrink-0" />
                      : <File className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
                    }
                    <span className="flex-1 text-foreground/80 truncate">{file.name}{file.is_dir ? "/" : ""}</span>
                    {!file.is_dir && (
                      <span className="text-muted-foreground/60 shrink-0">{formatFileSize(file.size)}</span>
                    )}
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* SKILL.md content */}
          <div className="border-t border-border pt-4">
            <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">SKILL.md</h3>
            <pre className="text-xs text-foreground/80 whitespace-pre-wrap leading-relaxed bg-secondary/30 rounded-lg p-4">
              {selectedSkill.content}
            </pre>
          </div>
        </div>
      </div>
    )
  }

  // ── Skills List View ───────────────────────────────────────────
  return (
    <div className="flex-1 min-h-0 overflow-y-auto p-6">
      <h2 className="text-lg font-semibold text-foreground mb-1">
        {t("settings.skills")}
      </h2>
      <p className="text-xs text-muted-foreground mb-4">
        {t("settings.skillsDesc")}
      </p>

      {/* Skill directories */}
      <div className="mb-5">
        <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">
          {t("settings.skillsDirs")}
        </h3>
        <div className="space-y-1">
          {/* Default directory (clickable) */}
          <button
            className="flex items-center gap-2 px-3 py-2 rounded-lg bg-secondary/30 text-xs w-full text-left hover:bg-secondary/50 transition-colors"
            onClick={() => handleOpenDir("~/.opencomputer/skills/")}
          >
            <FolderOpen className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
            <code className="flex-1 text-foreground/80 truncate">~/.opencomputer/skills/</code>
            <span className="text-[10px] px-1.5 py-0.5 rounded bg-secondary text-muted-foreground font-medium shrink-0">
              {t("settings.skillsDirDefault")}
            </span>
          </button>

          {/* Extra directories (clickable) */}
          {extraDirs.map((dir) => (
            <div key={dir} className="flex items-center gap-2 px-3 py-2 rounded-lg bg-secondary/30 text-xs group">
              <button
                className="flex items-center gap-2 flex-1 min-w-0 text-left hover:text-foreground transition-colors"
                onClick={() => handleOpenDir(dir)}
              >
                <FolderOpen className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
                <code className="flex-1 text-foreground/80 truncate" title={dir}>{dir}</code>
              </button>
              <button
                className="text-muted-foreground/50 hover:text-destructive transition-colors shrink-0 opacity-0 group-hover:opacity-100"
                onClick={() => handleRemoveDir(dir)}
                title={t("settings.skillsDirRemove")}
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </div>
          ))}
        </div>

        {/* Import directory button */}
        <button
          className="mt-2 flex items-center gap-1.5 text-xs text-primary hover:text-primary/80 transition-colors px-3 py-1.5"
          onClick={handleAddDir}
        >
          <FolderOpen className="h-3.5 w-3.5" />
          <span>{t("settings.skillsDirAdd")}</span>
        </button>
      </div>

      {/* Divider */}
      <div className="border-t border-border mb-4" />

      {/* Skills list */}
      <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">
        {t("settings.skillsList")}
        {!loading && skills.length > 0 && (
          <span className="ml-1.5 text-muted-foreground/60 font-normal normal-case">({skills.length})</span>
        )}
      </h3>

      {loading ? (
        <div className="flex items-center justify-center py-12">
          <div className="animate-spin h-5 w-5 border-2 border-foreground border-t-transparent rounded-full" />
        </div>
      ) : skills.length === 0 ? (
        <div className="text-center py-12">
          <Puzzle className="h-10 w-10 text-muted-foreground/30 mx-auto mb-3" />
          <p className="text-sm text-muted-foreground">{t("settings.noSkills")}</p>
          <p className="text-xs text-muted-foreground/70 mt-1">{t("settings.noSkillsHint")}</p>
        </div>
      ) : (
        <div className="space-y-1">
          {skills.map((skill) => (
            <div
              key={skill.name}
              className={cn(
                "flex items-center gap-3 w-full px-3 py-2.5 rounded-lg text-sm transition-colors group",
                skill.enabled
                  ? "text-foreground hover:bg-secondary/60"
                  : "text-muted-foreground/50 hover:bg-secondary/40"
              )}
            >
              {/* Toggle */}
              <Switch
                checked={skill.enabled}
                onCheckedChange={(v) => handleToggleSkill(skill.name, v)}
                onClick={(e) => e.stopPropagation()}
              />

              {/* Name + description (clickable → detail) */}
              <button
                className="flex-1 text-left min-w-0"
                onClick={() => handleSelectSkill(skill.name)}
              >
                <div className={cn("font-medium truncate", !skill.enabled && "line-through")}>{skill.name}</div>
                <div className="text-xs text-muted-foreground truncate">{skill.description}</div>
              </button>

              {/* Source tag */}
              <span className="text-[10px] px-1.5 py-0.5 rounded bg-secondary text-muted-foreground font-medium shrink-0">
                {skill.source}
              </span>

              {/* Open directory */}
              <button
                className="shrink-0 text-muted-foreground/40 hover:text-muted-foreground transition-colors opacity-0 group-hover:opacity-100"
                onClick={(e) => { e.stopPropagation(); handleOpenDir(skill.base_dir) }}
                title={skill.base_dir}
              >
                <FolderOpen className="h-3.5 w-3.5" />
              </button>

              <ChevronRight
                className="h-4 w-4 text-muted-foreground/30 shrink-0 group-hover:text-muted-foreground/60 transition-colors cursor-pointer"
                onClick={() => handleSelectSkill(skill.name)}
              />
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

// ── User Profile Panel ───────────────────────────────────────────

interface UserConfig {
  name?: string | null
  avatar?: string | null
  role?: string | null
  timezone?: string | null
  language?: string | null
  aiExperience?: string | null
  responseStyle?: string | null
  customInfo?: string | null
}

// Common timezones grouped by region
const TIMEZONE_OPTIONS = [
  { group: "Asia", zones: ["Asia/Shanghai", "Asia/Tokyo", "Asia/Seoul", "Asia/Singapore", "Asia/Hong_Kong", "Asia/Taipei", "Asia/Kolkata", "Asia/Dubai", "Asia/Bangkok"] },
  { group: "Americas", zones: ["America/New_York", "America/Chicago", "America/Denver", "America/Los_Angeles", "America/Toronto", "America/Sao_Paulo", "America/Mexico_City"] },
  { group: "Europe", zones: ["Europe/London", "Europe/Paris", "Europe/Berlin", "Europe/Moscow", "Europe/Istanbul", "Europe/Amsterdam", "Europe/Madrid"] },
  { group: "Pacific", zones: ["Pacific/Auckland", "Australia/Sydney", "Australia/Melbourne", "Pacific/Honolulu"] },
  { group: "Other", zones: ["UTC"] },
]

const LANGUAGE_OPTIONS = [
  { code: "zh-CN", label: "简体中文" },
  { code: "zh-TW", label: "繁體中文" },
  { code: "en", label: "English" },
  { code: "ja", label: "日本語" },
  { code: "ko", label: "한국어" },
  { code: "es", label: "Español" },
  { code: "pt", label: "Português" },
  { code: "ru", label: "Русский" },
  { code: "ar", label: "العربية" },
  { code: "tr", label: "Türkçe" },
  { code: "vi", label: "Tiếng Việt" },
  { code: "ms", label: "Bahasa Melayu" },
]

const PRESET_STYLES = ["concise", "detailed"]

function UserProfilePanel() {
  const { t, i18n } = useTranslation()
  const [config, setConfig] = useState<UserConfig>({})
  const [saving, setSaving] = useState(false)
  const [saved, setSaved] = useState(false)
  const [customStyle, setCustomStyle] = useState(false)

  useEffect(() => {
    // Load config + detect system defaults
    Promise.all([
      invoke<UserConfig>("get_user_config"),
      invoke<string>("get_system_timezone").catch(() => "UTC"),
    ]).then(([cfg, sysTz]) => {
      // Apply system defaults for empty fields
      if (!cfg.timezone) cfg.timezone = sysTz
      if (!cfg.language) {
        const lang = i18n.language
        const matched = LANGUAGE_OPTIONS.find((l) => lang.startsWith(l.code))
        if (matched) cfg.language = matched.code
      }
      setConfig(cfg)
      // Detect if responseStyle is custom (not a preset)
      if (cfg.responseStyle && !PRESET_STYLES.includes(cfg.responseStyle)) {
        setCustomStyle(true)
      }
    }).catch(console.error)
  }, [i18n.language])

  const handleSave = async () => {
    setSaving(true)
    try {
      await invoke("save_user_config", { config })
      setSaved(true)
      setTimeout(() => setSaved(false), 2000)
    } catch (e) {
      console.error(e)
    } finally {
      setSaving(false)
    }
  }

  const update = (patch: Partial<UserConfig>) => {
    setConfig((prev) => ({ ...prev, ...patch }))
  }

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="max-w-lg mx-auto py-8 px-6 space-y-8">

        {/* ── Header: Avatar + Name + Role ── */}
        <div className="flex items-start gap-5">
          {/* Avatar */}
          <div className="relative group shrink-0">
            <div className="w-16 h-16 rounded-2xl bg-gradient-to-br from-primary/20 via-primary/5 to-secondary border border-border/60 flex items-center justify-center overflow-hidden shadow-sm">
              {config.avatar ? (
                <img src={config.avatar} className="w-full h-full object-cover" alt="" />
              ) : (
                <User className="h-7 w-7 text-muted-foreground/50" />
              )}
            </div>
            <button
              className="absolute -bottom-1 -right-1 w-6 h-6 rounded-full bg-secondary border border-border flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity shadow-sm"
              onClick={() => {
                // TODO: file picker for avatar
              }}
              title={t("settings.profileAvatarChange")}
            >
              <Palette className="h-3 w-3 text-muted-foreground" />
            </button>
          </div>

          {/* Name + Role */}
          <div className="flex-1 space-y-2.5 pt-0.5">
            <input
              className="w-full text-base font-semibold text-foreground bg-transparent border-0 border-b border-transparent hover:border-border focus:border-primary/50 outline-none py-0.5 transition-colors placeholder:text-muted-foreground/40 placeholder:font-normal"
              value={config.name ?? ""}
              onChange={(e) => update({ name: e.target.value || null })}
              placeholder={t("settings.profileNamePlaceholder")}
            />
            <input
              className="w-full text-sm text-muted-foreground bg-transparent border-0 border-b border-transparent hover:border-border focus:border-primary/50 outline-none py-0.5 transition-colors placeholder:text-muted-foreground/30"
              value={config.role ?? ""}
              onChange={(e) => update({ role: e.target.value || null })}
              placeholder={t("settings.profileRolePlaceholder")}
            />
          </div>
        </div>

        {/* ── AI Experience Section ── */}
        <fieldset className="space-y-3">
          <legend className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-1">
            {t("settings.profileAiExperience")}
          </legend>

          <div className="flex gap-1.5">
            {(["expert", "intermediate", "beginner"] as const).map((level) => (
              <button
                key={level}
                className={cn(
                  "flex-1 px-3 py-2 rounded-lg text-xs transition-colors border",
                  config.aiExperience === level
                    ? "bg-primary/10 text-primary border-primary/20 font-medium"
                    : "text-foreground border-border/40 hover:bg-secondary/80"
                )}
                onClick={() => update({ aiExperience: config.aiExperience === level ? null : level })}
              >
                {t(`settings.profileAiExp${level.charAt(0).toUpperCase() + level.slice(1)}`)}
              </button>
            ))}
          </div>
        </fieldset>

        {/* ── Locale Section ── */}
        <fieldset className="space-y-4">
          <legend className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-1">
            {t("settings.profileTimezone")} & {t("settings.profileLanguage")}
          </legend>

          <div className="grid grid-cols-2 gap-3">
            {/* Timezone */}
            <div className="space-y-1.5">
              <label className="text-xs text-muted-foreground">{t("settings.profileTimezone")}</label>
              <select
                className="w-full px-2.5 py-1.5 text-xs bg-secondary/40 border border-border/40 rounded-lg text-foreground focus:outline-none focus:ring-1 focus:ring-primary/30 transition-colors"
                value={config.timezone ?? ""}
                onChange={(e) => update({ timezone: e.target.value || null })}
              >
                <option value="">—</option>
                {TIMEZONE_OPTIONS.map((group) => (
                  <optgroup key={group.group} label={group.group}>
                    {group.zones.map((tz) => (
                      <option key={tz} value={tz}>{tz.replace(/_/g, " ")}</option>
                    ))}
                  </optgroup>
                ))}
              </select>
            </div>

            {/* Language */}
            <div className="space-y-1.5">
              <label className="text-xs text-muted-foreground">{t("settings.profileLanguage")}</label>
              <select
                className="w-full px-2.5 py-1.5 text-xs bg-secondary/40 border border-border/40 rounded-lg text-foreground focus:outline-none focus:ring-1 focus:ring-primary/30 transition-colors"
                value={config.language ?? ""}
                onChange={(e) => update({ language: e.target.value || null })}
              >
                <option value="">—</option>
                {LANGUAGE_OPTIONS.map((lang) => (
                  <option key={lang.code} value={lang.code}>{lang.label}</option>
                ))}
              </select>
            </div>
          </div>
        </fieldset>

        {/* ── Response Style ── */}
        <fieldset className="space-y-3">
          <legend className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-1">
            {t("settings.profileResponseStyle")}
          </legend>

          <div className="flex gap-1.5">
            {PRESET_STYLES.map((style) => (
              <button
                key={style}
                className={cn(
                  "flex-1 px-3 py-2 rounded-lg text-xs transition-colors border",
                  !customStyle && config.responseStyle === style
                    ? "bg-primary/10 text-primary border-primary/20 font-medium"
                    : "text-foreground border-border/40 hover:bg-secondary/80"
                )}
                onClick={() => {
                  setCustomStyle(false)
                  update({ responseStyle: config.responseStyle === style ? null : style })
                }}
              >
                {t(`settings.profileStyle${style.charAt(0).toUpperCase() + style.slice(1)}`)}
              </button>
            ))}
            <button
              className={cn(
                "flex-1 px-3 py-2 rounded-lg text-xs transition-colors border",
                customStyle
                  ? "bg-primary/10 text-primary border-primary/20 font-medium"
                  : "text-foreground border-border/40 hover:bg-secondary/80"
              )}
              onClick={() => {
                setCustomStyle(true)
                if (!customStyle) update({ responseStyle: "" })
              }}
            >
              {t("settings.profileStyleCustom")}
            </button>
          </div>

          {customStyle && (
            <textarea
              className="w-full px-3 py-2 text-xs bg-secondary/30 border border-border/40 rounded-lg text-foreground placeholder:text-muted-foreground/40 focus:outline-none focus:ring-1 focus:ring-primary/30 transition-colors resize-none"
              rows={2}
              value={config.responseStyle ?? ""}
              onChange={(e) => update({ responseStyle: e.target.value || null })}
              placeholder={t("settings.profileStyleCustomPlaceholder")}
            />
          )}
        </fieldset>

        {/* ── Custom Info ── */}
        <fieldset className="space-y-2">
          <legend className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-1">
            {t("settings.profileCustomInfo")}
          </legend>
          <textarea
            className="w-full px-3 py-2.5 text-sm bg-secondary/30 border border-border/40 rounded-lg text-foreground placeholder:text-muted-foreground/30 focus:outline-none focus:ring-1 focus:ring-primary/30 transition-colors resize-none leading-relaxed"
            rows={3}
            value={config.customInfo ?? ""}
            onChange={(e) => update({ customInfo: e.target.value || null })}
            placeholder={t("settings.profileCustomInfoPlaceholder")}
          />
        </fieldset>

        {/* ── Save ── */}
        <div className="flex items-center gap-3 pt-2 pb-4">
          <button
            className={cn(
              "px-5 py-2 text-sm font-medium rounded-lg transition-all",
              saved
                ? "bg-green-500/10 text-green-600 border border-green-500/20"
                : "bg-primary text-primary-foreground hover:bg-primary/90 shadow-sm"
            )}
            onClick={handleSave}
            disabled={saving}
          >
            {saving ? t("common.saving") : saved ? (
              <span className="flex items-center gap-1.5">
                <Check className="h-3.5 w-3.5" />
                {t("settings.profileSaved")}
              </span>
            ) : t("common.save")}
          </button>
        </div>

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
  initialSection,
}: {
  onBack: () => void
  onCodexAuth: () => Promise<void>
  onCodexReauth?: () => void
  initialSection?: SettingsSection
}) {
  const { t } = useTranslation()
  const [activeSection, setActiveSection] =
    useState<SettingsSection>(initialSection ?? "providers")
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
        <div className="flex-1 flex flex-col min-h-0 overflow-hidden">
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
          {activeSection === "skills" && <SkillsPanel />}
          {activeSection === "profile" && <UserProfilePanel />}
          {activeSection === "appearance" && <AppearancePanel />}
          {activeSection === "language" && <LanguagePanel />}
          {activeSection === "about" && <AboutPanel />}
        </div>
      </div>
    </div>
  )
}
