import { useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Monitor } from "lucide-react"

import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { TIMEZONE_OPTIONS, type UserConfig } from "@/components/settings/profile-panel/types"
import AvatarSection from "@/components/settings/profile-panel/AvatarSection"
import { useAvatarUpload } from "@/hooks/useAvatarUpload"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"

import type { OnboardingDraft } from "../types"

interface ProfileStepProps {
  draft: OnboardingDraft["profile"]
  onChange: (patch: OnboardingDraft["profile"]) => void
}

const EXPERIENCE_OPTIONS: Array<{ id: "beginner" | "intermediate" | "expert"; labelKey: string }> =
  [
    { id: "beginner", labelKey: "onboarding.profile.experience.beginner" },
    { id: "intermediate", labelKey: "onboarding.profile.experience.intermediate" },
    { id: "expert", labelKey: "onboarding.profile.experience.expert" },
  ]

const STYLE_OPTIONS: Array<{ id: "concise" | "balanced" | "detailed"; labelKey: string }> = [
  { id: "concise", labelKey: "onboarding.profile.style.concise" },
  { id: "balanced", labelKey: "onboarding.profile.style.balanced" },
  { id: "detailed", labelKey: "onboarding.profile.style.detailed" },
]

/**
 * Step 3 — basic profile. All four fields are optional; empty means
 * "leave the existing UserConfig value alone" (apply helper treats empty
 * string as None).
 */
export function ProfileStep({ draft, onChange }: ProfileStepProps) {
  const { t } = useTranslation()
  const [name, setName] = useState(draft?.name ?? "")
  const [timezone, setTimezone] = useState(draft?.timezone ?? "")
  const [experience, setExperience] = useState(draft?.aiExperience ?? "")
  const [style, setStyle] = useState(draft?.responseStyle ?? "")
  const [avatar, setAvatar] = useState<string | null>(null)
  const userConfigRef = useRef<UserConfig | null>(null)

  useEffect(() => {
    void (async () => {
      try {
        const cfg = await getTransport().call<UserConfig>("get_user_config")
        userConfigRef.current = cfg
        if (cfg.avatar) setAvatar(cfg.avatar)
      } catch (e) {
        logger.warn("onboarding", "ProfileStep::loadAvatar", "get_user_config failed", e)
      }
    })()
  }, [])

  // Avatar writes are side-channel (no "Next" step applies them later):
  // save_avatar lands bytes on disk, then save_user_config persists the
  // path so early exits and settings panels stay in sync.
  const { cropSrc, handleAvatarPick, handleCropCancel, handleCropConfirm } = useAvatarUpload({
    fileName: () => `user_${Date.now()}.png`,
    logCategory: "onboarding.ProfileStep",
    onSaved: async (path) => {
      setAvatar(path)
      const next: UserConfig = { ...(userConfigRef.current ?? {}), avatar: path }
      userConfigRef.current = next
      try {
        await getTransport().call("save_user_config", { config: next })
      } catch (e) {
        logger.error("onboarding", "ProfileStep::saveAvatar", "save_user_config failed", e)
      }
    },
  })

  const systemTimezone = (() => {
    try {
      return Intl.DateTimeFormat().resolvedOptions().timeZone || ""
    } catch {
      return ""
    }
  })()

  useEffect(() => {
    onChange({
      name,
      timezone,
      aiExperience: experience,
      responseStyle: style,
    })
  }, [name, timezone, experience, style]) // eslint-disable-line react-hooks/exhaustive-deps

  return (
    <div className="px-6 py-6 space-y-6 max-w-xl mx-auto">
      <div className="text-center space-y-1">
        <h2 className="text-xl font-semibold">{t("onboarding.profile.title")}</h2>
        <p className="text-sm text-muted-foreground">{t("onboarding.profile.subtitle")}</p>
      </div>

      <AvatarSection
        avatar={avatar}
        cropSrc={cropSrc}
        onAvatarPick={handleAvatarPick}
        onCropConfirm={handleCropConfirm}
        onCropCancel={handleCropCancel}
      />

      <div className="grid gap-4">
        <div className="space-y-1">
          <Label htmlFor="onb-name">{t("onboarding.profile.name")}</Label>
          <Input
            id="onb-name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder={t("onboarding.profile.namePlaceholder")}
          />
        </div>

        <div className="space-y-1">
          <Label>{t("onboarding.profile.timezone")}</Label>
          <button
            type="button"
            onClick={() => setTimezone("")}
            className={`flex items-center gap-2 w-full px-3 py-2 rounded-md border text-sm transition-colors ${
              timezone === ""
                ? "border-primary bg-primary/10 text-primary"
                : "border-border hover:border-foreground/30"
            }`}
          >
            <Monitor className="h-3.5 w-3.5 opacity-60" />
            <span className="flex-1 text-left">
              {t("settings.profileTimezoneSystem")}
              {systemTimezone && (
                <span className="text-xs text-muted-foreground ml-1">({systemTimezone})</span>
              )}
            </span>
          </button>
          <Select value={timezone} onValueChange={(v) => setTimezone(v)}>
            <SelectTrigger className="mt-1">
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

        <div className="space-y-1">
          <Label>{t("onboarding.profile.experience.label")}</Label>
          <div className="flex gap-2 flex-wrap">
            {EXPERIENCE_OPTIONS.map((opt) => (
              <button
                key={opt.id}
                type="button"
                onClick={() => setExperience(experience === opt.id ? "" : opt.id)}
                className={`rounded-md border px-3 py-1.5 text-sm transition-colors ${
                  experience === opt.id
                    ? "border-primary bg-primary/10 text-primary"
                    : "border-border hover:border-foreground/30"
                }`}
              >
                {t(opt.labelKey)}
              </button>
            ))}
          </div>
        </div>

        <div className="space-y-1">
          <Label>{t("onboarding.profile.style.label")}</Label>
          <div className="flex gap-2 flex-wrap">
            {STYLE_OPTIONS.map((opt) => (
              <button
                key={opt.id}
                type="button"
                onClick={() => setStyle(style === opt.id ? "" : opt.id)}
                className={`rounded-md border px-3 py-1.5 text-sm transition-colors ${
                  style === opt.id
                    ? "border-primary bg-primary/10 text-primary"
                    : "border-border hover:border-foreground/30"
                }`}
              >
                {t(opt.labelKey)}
              </button>
            ))}
          </div>
        </div>
      </div>
    </div>
  )
}
