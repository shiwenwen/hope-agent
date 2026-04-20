import { useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"

import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"

import type { OnboardingDraft } from "../types"

interface ProfileStepProps {
  draft: OnboardingDraft["profile"]
  onChange: (patch: OnboardingDraft["profile"]) => void
}

const EXPERIENCE_OPTIONS: Array<{ id: "beginner" | "intermediate" | "expert"; labelKey: string }> = [
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

  const systemTimezone = useMemo(() => {
    try {
      return Intl.DateTimeFormat().resolvedOptions().timeZone || ""
    } catch {
      return ""
    }
  }, [])

  // Default to system timezone on first render when draft didn't provide one.
  useEffect(() => {
    if (!timezone && systemTimezone) setTimezone(systemTimezone)
  }, [systemTimezone]) // eslint-disable-line react-hooks/exhaustive-deps

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
          <Label htmlFor="onb-tz">{t("onboarding.profile.timezone")}</Label>
          <Input
            id="onb-tz"
            value={timezone}
            onChange={(e) => setTimezone(e.target.value)}
            placeholder={systemTimezone || "Asia/Shanghai"}
          />
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
