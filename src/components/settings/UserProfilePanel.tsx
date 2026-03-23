import { useState, useEffect, useRef } from "react"
import { invoke, convertFileSrc } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { logger } from "@/lib/logger"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { AvatarCropDialog } from "@/components/settings/AvatarCropDialog"
import { Camera, Check, Monitor } from "lucide-react"

interface UserConfig {
  name?: string | null
  avatar?: string | null
  gender?: string | null
  birthday?: string | null
  role?: string | null
  timezone?: string | null
  language?: string | null
  aiExperience?: string | null
  responseStyle?: string | null
  customInfo?: string | null
}

const GENDER_PRESETS = ["male", "female"]

// Common timezones grouped by region, with i18n display names
const TIMEZONE_OPTIONS: { groupKey: string; zones: { value: string; labelKey: string }[] }[] = [
  {
    groupKey: "Asia",
    zones: [
      { value: "Asia/Shanghai", labelKey: "tz.shanghai" },
      { value: "Asia/Tokyo", labelKey: "tz.tokyo" },
      { value: "Asia/Seoul", labelKey: "tz.seoul" },
      { value: "Asia/Singapore", labelKey: "tz.singapore" },
      { value: "Asia/Hong_Kong", labelKey: "tz.hongkong" },
      { value: "Asia/Taipei", labelKey: "tz.taipei" },
      { value: "Asia/Kolkata", labelKey: "tz.kolkata" },
      { value: "Asia/Dubai", labelKey: "tz.dubai" },
      { value: "Asia/Bangkok", labelKey: "tz.bangkok" },
    ],
  },
  {
    groupKey: "Americas",
    zones: [
      { value: "America/New_York", labelKey: "tz.newyork" },
      { value: "America/Chicago", labelKey: "tz.chicago" },
      { value: "America/Denver", labelKey: "tz.denver" },
      { value: "America/Los_Angeles", labelKey: "tz.losangeles" },
      { value: "America/Toronto", labelKey: "tz.toronto" },
      { value: "America/Sao_Paulo", labelKey: "tz.saopaulo" },
      { value: "America/Mexico_City", labelKey: "tz.mexicocity" },
    ],
  },
  {
    groupKey: "Europe",
    zones: [
      { value: "Europe/London", labelKey: "tz.london" },
      { value: "Europe/Paris", labelKey: "tz.paris" },
      { value: "Europe/Berlin", labelKey: "tz.berlin" },
      { value: "Europe/Moscow", labelKey: "tz.moscow" },
      { value: "Europe/Istanbul", labelKey: "tz.istanbul" },
      { value: "Europe/Amsterdam", labelKey: "tz.amsterdam" },
      { value: "Europe/Madrid", labelKey: "tz.madrid" },
    ],
  },
  {
    groupKey: "Pacific",
    zones: [
      { value: "Pacific/Auckland", labelKey: "tz.auckland" },
      { value: "Australia/Sydney", labelKey: "tz.sydney" },
      { value: "Australia/Melbourne", labelKey: "tz.melbourne" },
      { value: "Pacific/Honolulu", labelKey: "tz.honolulu" },
    ],
  },
  { groupKey: "Other", zones: [{ value: "UTC", labelKey: "tz.utc" }] },
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

export default function UserProfilePanel({ onSaved }: { onSaved?: () => void } = {}) {
  const { t, i18n } = useTranslation()
  const [config, setConfig] = useState<UserConfig>({})
  const [saving, setSaving] = useState(false)
  const [saved, setSaved] = useState(false)
  const [customStyle, setCustomStyle] = useState(false)
  const [customGender, setCustomGender] = useState(false)
  const composingRef = useRef(false)
  const [cropSrc, setCropSrc] = useState<string | null>(null)

  useEffect(() => {
    Promise.all([
      invoke<UserConfig>("get_user_config"),
      invoke<string>("get_system_timezone").catch(() => "UTC"),
    ])
      .then(([cfg, sysTz]) => {
        if (!cfg.timezone) cfg.timezone = sysTz
        if (!cfg.language) {
          const matched = LANGUAGE_OPTIONS.find((l) => i18n.language.startsWith(l.code))
          if (matched) cfg.language = matched.code
        }
        setConfig(cfg)
        if (cfg.responseStyle && !PRESET_STYLES.includes(cfg.responseStyle)) {
          setCustomStyle(true)
        }
        if (cfg.gender && !GENDER_PRESETS.includes(cfg.gender)) {
          setCustomGender(true)
        }
      })
      .catch((e: unknown) =>
        logger.error("settings", "UserProfilePanel::load", "Failed to load user config", e),
      )
  }, [i18n.language])

  const handleSave = async () => {
    setSaving(true)
    try {
      await invoke("save_user_config", { config })
      setSaved(true)
      onSaved?.()
      setTimeout(() => setSaved(false), 2000)
    } catch (e) {
      logger.error("settings", "UserProfilePanel::save", "Failed to save user config", e)
    } finally {
      setSaving(false)
    }
  }

  const update = (patch: Partial<UserConfig>) => {
    setConfig((prev) => ({ ...prev, ...patch }))
  }

  /** Props for text inputs that handle IME composition correctly */
  const textInputProps = (field: keyof UserConfig) => ({
    value: (config[field] as string) ?? "",
    onChange: (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => {
      // During IME composing, keep raw value; on blur, normalize empty to null
      update({ [field]: e.target.value })
    },
    onCompositionStart: () => {
      composingRef.current = true
    },
    onCompositionEnd: (e: React.CompositionEvent<HTMLInputElement | HTMLTextAreaElement>) => {
      composingRef.current = false
      update({ [field]: (e.target as HTMLInputElement).value })
    },
    onBlur: (e: React.FocusEvent<HTMLInputElement | HTMLTextAreaElement>) => {
      // Normalize: empty string → null for clean storage
      if (!e.target.value) update({ [field]: null })
    },
  })

  const handleAvatarPick = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog")
      const selected = await open({
        filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "gif", "webp", "svg"] }],
        multiple: false,
      })
      if (selected) {
        setCropSrc(convertFileSrc(selected as string))
      }
    } catch (e) {
      logger.error("settings", "UserProfilePanel::pickAvatar", "Failed to pick avatar", e)
    }
  }

  const handleCropConfirm = async (blob: Blob) => {
    setCropSrc(null)
    try {
      const buf = await blob.arrayBuffer()
      const bytes = new Uint8Array(buf)
      let binary = ""
      for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i])
      const base64 = window.btoa(binary)
      const fileName = `user_${Date.now()}.png`
      const savedPath = await invoke<string>("save_avatar", { imageData: base64, fileName })
      update({ avatar: savedPath })
    } catch (e) {
      logger.error("settings", "UserProfilePanel::saveAvatar", "Failed to save avatar", e)
    }
  }

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-hidden">
      {/* Scrollable content */}
      <div className="flex-1 overflow-y-auto p-6">
        <div className="max-w-4xl">
          <h2 className="text-lg font-semibold text-foreground mb-1">{t("settings.profile")}</h2>
          <p className="text-xs text-muted-foreground mb-5">{t("settings.profileDesc")}</p>

          <div className="space-y-5">
            {/* ── Avatar ── */}
            <div
              className="flex flex-col items-center gap-2 py-4 cursor-pointer"
              onClick={handleAvatarPick}
            >
              <div className="w-16 h-16 rounded-full bg-secondary border border-border/50 flex items-center justify-center overflow-hidden hover:border-primary/30 transition-colors">
                {config.avatar ? (
                  <img
                    src={
                      config.avatar.startsWith("/") ? convertFileSrc(config.avatar) : config.avatar
                    }
                    className="w-full h-full object-cover"
                    alt=""
                  />
                ) : (
                  <Camera className="h-5 w-5 text-muted-foreground/40" />
                )}
              </div>
              <span className="text-xs text-muted-foreground">
                {t("settings.profileAvatarChange")}
              </span>
            </div>

            {/* Avatar crop dialog */}
            {cropSrc && (
              <AvatarCropDialog
                open={!!cropSrc}
                imageSrc={cropSrc}
                onConfirm={handleCropConfirm}
                onCancel={() => setCropSrc(null)}
              />
            )}

            {/* ── Name ── */}
            <div>
              <div className="text-xs font-medium text-muted-foreground mb-2 px-1">
                {t("settings.profileName")}
              </div>
              <Input
                className="bg-secondary/40 rounded-lg"
                {...textInputProps("name")}
                placeholder={t("settings.profileNamePlaceholder")}
              />
            </div>

            {/* ── Gender ── */}
            <div>
              <div className="text-xs font-medium text-muted-foreground mb-2 px-1">
                {t("settings.profileGender")}
              </div>
              <div className="space-y-0.5">
                {GENDER_PRESETS.map((g) => (
                  <button
                    key={g}
                    className={cn(
                      "flex items-center gap-3 w-full px-3 py-2.5 rounded-lg text-sm transition-colors",
                      !customGender && config.gender === g
                        ? "bg-primary/10 text-primary font-medium"
                        : "bg-secondary/20 text-foreground hover:bg-secondary/60",
                    )}
                    onClick={() => {
                      setCustomGender(false)
                      update({ gender: config.gender === g ? null : g })
                    }}
                  >
                    <span className="flex-1 text-left">
                      {t(`settings.profileGender${g.charAt(0).toUpperCase() + g.slice(1)}`)}
                    </span>
                    {!customGender && config.gender === g && (
                      <Check className="h-4 w-4 text-primary shrink-0" />
                    )}
                  </button>
                ))}
                <button
                  className={cn(
                    "flex items-center gap-3 w-full px-3 py-2.5 rounded-lg text-sm transition-colors",
                    customGender
                      ? "bg-primary/10 text-primary font-medium"
                      : "bg-secondary/20 text-foreground hover:bg-secondary/60",
                  )}
                  onClick={() => {
                    setCustomGender(true)
                    if (!customGender) update({ gender: "" })
                  }}
                >
                  <span className="flex-1 text-left">{t("settings.profileGenderCustom")}</span>
                  {customGender && <Check className="h-4 w-4 text-primary shrink-0" />}
                </button>
              </div>
              {customGender && (
                <Input
                  className="mt-2 bg-secondary/40 rounded-lg"
                  {...textInputProps("gender")}
                  placeholder={t("settings.profileGenderCustomPlaceholder")}
                />
              )}
            </div>

            {/* ── Birthday ── */}
            <div>
              <div className="text-xs font-medium text-muted-foreground mb-2 px-1">
                {t("settings.profileBirthday")}
              </div>
              <Input
                type="date"
                className="bg-secondary/40 rounded-lg"
                value={config.birthday ?? ""}
                onChange={(e) => {
                  update({ birthday: e.target.value || null })
                }}
              />
              {config.birthday &&
                (() => {
                  const bd = new Date(config.birthday + "T00:00:00")
                  if (isNaN(bd.getTime())) return null
                  const today = new Date()
                  let age = today.getFullYear() - bd.getFullYear()
                  const hadBirthdayThisYear =
                    today.getMonth() > bd.getMonth() ||
                    (today.getMonth() === bd.getMonth() && today.getDate() >= bd.getDate())
                  if (!hadBirthdayThisYear) age -= 1
                  const isBirthday =
                    today.getMonth() === bd.getMonth() && today.getDate() === bd.getDate()
                  return (
                    <div className="mt-2 px-1 flex items-center gap-2">
                      <span className="text-xs text-muted-foreground">
                        {t("settings.profileAgeDisplay", { age })}
                      </span>
                      {isBirthday && (
                        <span className="text-xs font-medium text-amber-500 animate-pulse">
                          🎂 {t("settings.profileBirthdaySurprise")}
                        </span>
                      )}
                    </div>
                  )
                })()}
            </div>

            {/* ── Role ── */}
            <div>
              <div className="text-xs font-medium text-muted-foreground mb-2 px-1">
                {t("settings.profileRole")}
              </div>
              <Input
                className="bg-secondary/40 rounded-lg"
                {...textInputProps("role")}
                placeholder={t("settings.profileRolePlaceholder")}
              />
            </div>

            {/* ── AI Experience ── */}
            <div>
              <div className="text-xs font-medium text-muted-foreground mb-2 px-1">
                {t("settings.profileAiExperience")}
              </div>
              <div className="space-y-0.5">
                {(["expert", "intermediate", "beginner"] as const).map((level) => (
                  <button
                    key={level}
                    className={cn(
                      "flex items-center gap-3 w-full px-3 py-2.5 rounded-lg text-sm transition-colors",
                      config.aiExperience === level
                        ? "bg-primary/10 text-primary font-medium"
                        : "bg-secondary/20 text-foreground hover:bg-secondary/60",
                    )}
                    onClick={() =>
                      update({ aiExperience: config.aiExperience === level ? null : level })
                    }
                  >
                    <span className="flex-1 text-left">
                      {t(`settings.profileAiExp${level.charAt(0).toUpperCase() + level.slice(1)}`)}
                    </span>
                    {config.aiExperience === level && (
                      <Check className="h-4 w-4 text-primary shrink-0" />
                    )}
                  </button>
                ))}
              </div>
            </div>

            <div className="border-t border-border/50" />

            {/* ── Timezone ── */}
            <div>
              <div className="text-xs font-medium text-muted-foreground mb-2 px-1">
                {t("settings.profileTimezone")}
              </div>
              <div className="space-y-0.5">
                <button
                  className={cn(
                    "flex items-center gap-3 w-full px-3 py-2.5 rounded-lg text-sm transition-colors",
                    !config.timezone
                      ? "bg-primary/10 text-primary font-medium"
                      : "bg-secondary/20 text-foreground hover:bg-secondary/60",
                  )}
                  onClick={() => update({ timezone: null })}
                >
                  <Monitor className="h-4 w-4 shrink-0 opacity-60" />
                  <span className="flex-1 text-left">{t("settings.profileTimezoneSystem")}</span>
                  {!config.timezone && <Check className="h-4 w-4 text-primary shrink-0" />}
                </button>
              </div>
              <Select
                value={config.timezone ?? ""}
                onValueChange={(v) => update({ timezone: v || null })}
              >
                <SelectTrigger className="mt-1 bg-secondary/20 text-sm hover:bg-secondary/60">
                  <SelectValue placeholder={t("settings.profileTimezoneSystem")} />
                </SelectTrigger>
                <SelectContent>
                  {TIMEZONE_OPTIONS.map((group) => (
                    <SelectGroup key={group.groupKey}>
                      <SelectLabel>{group.groupKey}</SelectLabel>
                      {group.zones.map((tz) => (
                        <SelectItem key={tz.value} value={tz.value}>
                          {t(tz.labelKey)}
                        </SelectItem>
                      ))}
                    </SelectGroup>
                  ))}
                </SelectContent>
              </Select>
            </div>

            {/* ── Language ── */}
            <div>
              <div className="text-xs font-medium text-muted-foreground mb-2 px-1">
                {t("settings.profileLanguage")}
              </div>
              <div className="space-y-0.5">
                <button
                  className={cn(
                    "flex items-center gap-3 w-full px-3 py-2.5 rounded-lg text-sm transition-colors",
                    !config.language
                      ? "bg-primary/10 text-primary font-medium"
                      : "bg-secondary/20 text-foreground hover:bg-secondary/60",
                  )}
                  onClick={() => update({ language: null })}
                >
                  <Monitor className="h-4 w-4 shrink-0 opacity-60" />
                  <span className="flex-1 text-left">{t("settings.profileLanguageSystem")}</span>
                  {!config.language && <Check className="h-4 w-4 text-primary shrink-0" />}
                </button>
              </div>
              <Select
                value={config.language ?? ""}
                onValueChange={(v) => update({ language: v || null })}
              >
                <SelectTrigger className="mt-1 bg-secondary/20 text-sm hover:bg-secondary/60">
                  <SelectValue placeholder={t("settings.profileLanguageSystem")} />
                </SelectTrigger>
                <SelectContent>
                  {LANGUAGE_OPTIONS.map((lang) => (
                    <SelectItem key={lang.code} value={lang.code}>
                      {lang.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <div className="border-t border-border/50" />

            {/* ── Response Style ── */}
            <div>
              <div className="text-xs font-medium text-muted-foreground mb-2 px-1">
                {t("settings.profileResponseStyle")}
              </div>
              <div className="space-y-0.5">
                {PRESET_STYLES.map((style) => (
                  <button
                    key={style}
                    className={cn(
                      "flex items-center gap-3 w-full px-3 py-2.5 rounded-lg text-sm transition-colors",
                      !customStyle && config.responseStyle === style
                        ? "bg-primary/10 text-primary font-medium"
                        : "bg-secondary/20 text-foreground hover:bg-secondary/60",
                    )}
                    onClick={() => {
                      setCustomStyle(false)
                      update({ responseStyle: config.responseStyle === style ? null : style })
                    }}
                  >
                    <span className="flex-1 text-left">
                      {t(`settings.profileStyle${style.charAt(0).toUpperCase() + style.slice(1)}`)}
                    </span>
                    {!customStyle && config.responseStyle === style && (
                      <Check className="h-4 w-4 text-primary shrink-0" />
                    )}
                  </button>
                ))}
                <button
                  className={cn(
                    "flex items-center gap-3 w-full px-3 py-2.5 rounded-lg text-sm transition-colors",
                    customStyle
                      ? "bg-primary/10 text-primary font-medium"
                      : "bg-secondary/20 text-foreground hover:bg-secondary/60",
                  )}
                  onClick={() => {
                    setCustomStyle(true)
                    if (!customStyle) update({ responseStyle: "" })
                  }}
                >
                  <span className="flex-1 text-left">{t("settings.profileStyleCustom")}</span>
                  {customStyle && <Check className="h-4 w-4 text-primary shrink-0" />}
                </button>
              </div>

              {customStyle && (
                <Textarea
                  className="mt-2 bg-secondary/40 rounded-lg resize-none leading-relaxed"
                  rows={4}
                  {...textInputProps("responseStyle")}
                  placeholder={t("settings.profileStyleCustomPlaceholder")}
                />
              )}
            </div>

            <div className="border-t border-border/50" />

            {/* ── Custom Info ── */}
            <div>
              <div className="text-xs font-medium text-muted-foreground mb-2 px-1">
                {t("settings.profileCustomInfo")}
              </div>
              <Textarea
                className="bg-secondary/40 rounded-lg resize-none leading-relaxed"
                rows={5}
                {...textInputProps("customInfo")}
                placeholder={t("settings.profileCustomInfoPlaceholder")}
              />
            </div>
          </div>
        </div>
      </div>

      {/* ── Save — fixed bottom-right ── */}
      <div className="shrink-0 flex justify-end px-6 py-3 border-t border-border/30">
        <Button
          className={cn(saved && "bg-green-500/10 text-green-600 hover:bg-green-500/20")}
          onClick={handleSave}
          disabled={saving}
        >
          {saving ? (
            t("common.saving")
          ) : saved ? (
            <span className="flex items-center gap-1.5">
              <Check className="h-3.5 w-3.5" />
              {t("settings.profileSaved")}
            </span>
          ) : (
            t("common.save")
          )}
        </Button>
      </div>
    </div>
  )
}
