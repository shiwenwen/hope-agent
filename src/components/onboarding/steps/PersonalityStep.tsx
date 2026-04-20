import { useTranslation } from "react-i18next"
import { Bot, Coffee, Palette, Wrench } from "lucide-react"

import type { PersonalityPresetId } from "../types"

interface PersonalityStepProps {
  selected: PersonalityPresetId
  onSelect: (id: PersonalityPresetId) => void
}

const PRESETS: Array<{ id: PersonalityPresetId; icon: React.ReactNode }> = [
  { id: "default", icon: <Bot className="h-5 w-5" /> },
  { id: "engineer", icon: <Wrench className="h-5 w-5" /> },
  { id: "creative", icon: <Palette className="h-5 w-5" /> },
  { id: "companion", icon: <Coffee className="h-5 w-5" /> },
]

/**
 * Step 4 — personality preset cards. Clicking selects; clicking again
 * deselects. The actual write to the default agent happens on Next via
 * `apply_personality_preset_cmd`, so switching mid-step has no cost.
 */
export function PersonalityStep({ selected, onSelect }: PersonalityStepProps) {
  const { t } = useTranslation()
  return (
    <div className="px-6 py-6 space-y-5 max-w-2xl mx-auto">
      <div className="text-center space-y-1">
        <h2 className="text-xl font-semibold">{t("onboarding.personality.title")}</h2>
        <p className="text-sm text-muted-foreground">{t("onboarding.personality.subtitle")}</p>
      </div>

      <div className="grid gap-3 sm:grid-cols-2">
        {PRESETS.map((preset) => {
          const isActive = selected === preset.id
          return (
            <button
              key={preset.id}
              type="button"
              onClick={() => onSelect(isActive ? "" : preset.id)}
              className={`text-left rounded-lg border-2 px-4 py-3 transition-all ${
                isActive
                  ? "border-primary bg-primary/5"
                  : "border-border hover:border-foreground/30"
              }`}
            >
              <div className="flex items-center gap-2 mb-1.5">
                <span
                  className={`flex h-8 w-8 items-center justify-center rounded-md ${
                    isActive ? "bg-primary/15 text-primary" : "bg-muted text-muted-foreground"
                  }`}
                >
                  {preset.icon}
                </span>
                <span className="font-medium">
                  {t(`onboarding.personality.presets.${preset.id}.name`)}
                </span>
              </div>
              <p className="text-sm text-muted-foreground leading-relaxed">
                {t(`onboarding.personality.presets.${preset.id}.desc`)}
              </p>
            </button>
          )
        })}
      </div>
    </div>
  )
}
