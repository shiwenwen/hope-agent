import { useState } from "react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { X } from "lucide-react"
import { OpenClawHintBanner } from "./CustomTab"
import { TONE_PRESETS } from "../types"
import type { AgentConfig, PersonalityConfig } from "../types"

interface PersonalityTabProps {
  config: AgentConfig
  persona: string
  openclawMode: boolean
  updatePersonality: (patch: Partial<PersonalityConfig>) => void
  setPersona: (v: string) => void
  textInputProps: (getter: string, setter: (v: string) => void) => {
    value: string
    onChange: (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => void
    onCompositionStart: () => void
    onCompositionEnd: (e: React.CompositionEvent<HTMLInputElement | HTMLTextAreaElement>) => void
  }
  CharCounter: React.ComponentType<{ value: string }>
}

export default function PersonalityTab({
  config,
  persona,
  openclawMode,
  updatePersonality,
  setPersona,
  textInputProps,
  CharCounter,
}: PersonalityTabProps) {
  const { t } = useTranslation()
  const [traitInput, setTraitInput] = useState("")
  const [principleInput, setPrincipleInput] = useState("")

  return (
    <div className="space-y-5">
      {openclawMode && <OpenClawHintBanner />}

      <div className={openclawMode ? "opacity-50 pointer-events-none space-y-5" : "space-y-5"}>
        {/* Vibe */}
        <div>
          <div className="text-xs font-medium text-muted-foreground mb-1 px-1">
            {t("settings.agentVibe")}
          </div>
          <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">
            {t("settings.agentVibeDesc")}
          </p>
          <Textarea
            className="bg-secondary/40 rounded-lg resize-y leading-relaxed min-h-[60px]"
            rows={3}

            {...textInputProps(config.personality.vibe ?? "", (v) =>
              updatePersonality({ vibe: v || null }),
            )}
            placeholder={t("settings.agentVibePlaceholder")}
          />
        </div>

        {/* Tone */}
        <div>
          <div className="text-xs font-medium text-muted-foreground mb-2 px-1">
            {t("settings.agentTone")}
          </div>
          <div className="flex flex-wrap gap-1.5 mb-2">
            {TONE_PRESETS.map((preset) => (
              <button
                key={preset.value}
    
                className={cn(
                  "px-2.5 py-1.5 text-xs rounded-md transition-colors",
                  config.personality.tone === preset.value
                    ? "bg-primary/10 text-primary font-medium"
                    : "bg-secondary/30 text-foreground hover:bg-secondary/60",
                )}
                onClick={() =>
                  updatePersonality({
                    tone: config.personality.tone === preset.value ? null : preset.value,
                  })
                }
              >
                {t(preset.labelKey)}
              </button>
            ))}
          </div>
          <Textarea
            className="bg-secondary/40 rounded-lg resize-y leading-relaxed min-h-[60px]"
            rows={3}

            {...textInputProps(config.personality.tone ?? "", (v) =>
              updatePersonality({ tone: v || null }),
            )}
            placeholder={t("settings.agentTonePlaceholder")}
          />
        </div>

        {/* Traits (tag input) */}
        <div>
          <div className="text-xs font-medium text-muted-foreground mb-1 px-1">
            {t("settings.agentTraits")}
          </div>
          <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">
            {t("settings.agentTraitsDesc")}
          </p>
          <div className="flex flex-wrap gap-1.5 mb-2">
            {config.personality.traits.map((trait) => (
              <span
                key={trait}
                className="inline-flex items-center gap-1 px-2 py-1 text-xs rounded-md bg-secondary text-foreground"
              >
                {trait}
                <button
      
                  className="text-muted-foreground hover:text-destructive transition-colors"
                  onClick={() =>
                    updatePersonality({
                      traits: config.personality.traits.filter((t) => t !== trait),
                    })
                  }
                >
                  <X className="h-3 w-3" />
                </button>
              </span>
            ))}
          </div>
          <Input
            className="bg-secondary/40 rounded-lg"

            value={traitInput}
            onChange={(e) => setTraitInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && traitInput.trim()) {
                const val = traitInput.trim()
                if (!config.personality.traits.includes(val)) {
                  updatePersonality({ traits: [...config.personality.traits, val] })
                }
                setTraitInput("")
              }
            }}
            placeholder={t("settings.agentTraitsPlaceholder")}
          />
        </div>

        <div className="border-t border-border/50" />

        {/* Principles (tag input) */}
        <div>
          <div className="text-xs font-medium text-muted-foreground mb-1 px-1">
            {t("settings.agentPrinciples")}
          </div>
          <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">
            {t("settings.agentPrinciplesDesc")}
          </p>
          <div className="space-y-1 mb-2">
            {config.personality.principles.map((p, i) => (
              <div
                key={i}
                className="flex items-center gap-2 px-2.5 py-1.5 text-xs rounded-md bg-secondary/30 text-foreground"
              >
                <span className="flex-1">{p}</span>
                <button
      
                  className="text-muted-foreground hover:text-destructive transition-colors shrink-0"
                  onClick={() =>
                    updatePersonality({
                      principles: config.personality.principles.filter((_, idx) => idx !== i),
                    })
                  }
                >
                  <X className="h-3 w-3" />
                </button>
              </div>
            ))}
          </div>
          <Textarea
            className="bg-secondary/40 rounded-lg resize-y leading-relaxed min-h-[50px]"
            rows={2}

            value={principleInput}
            onChange={(e) => setPrincipleInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey && principleInput.trim()) {
                e.preventDefault()
                updatePersonality({
                  principles: [...config.personality.principles, principleInput.trim()],
                })
                setPrincipleInput("")
              }
            }}
            placeholder={t("settings.agentPrinciplesPlaceholder")}
          />
        </div>

        <div className="border-t border-border/50" />

        {/* Boundaries */}
        <div>
          <div className="text-xs font-medium text-muted-foreground mb-1 px-1">
            {t("settings.agentBoundaries")}
          </div>
          <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">
            {t("settings.agentBoundariesDesc")}
          </p>
          <Textarea
            className="bg-secondary/40 rounded-lg resize-y leading-relaxed min-h-[60px]"
            rows={3}

            {...textInputProps(config.personality.boundaries ?? "", (v) =>
              updatePersonality({ boundaries: v || null }),
            )}
            placeholder={t("settings.agentBoundariesPlaceholder")}
          />
        </div>

        {/* Quirks */}
        <div>
          <div className="text-xs font-medium text-muted-foreground mb-1 px-1">
            {t("settings.agentQuirks")}
          </div>
          <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">
            {t("settings.agentQuirksDesc")}
          </p>
          <Textarea
            className="bg-secondary/40 rounded-lg resize-y leading-relaxed min-h-[60px]"
            rows={3}

            {...textInputProps(config.personality.quirks ?? "", (v) =>
              updatePersonality({ quirks: v || null }),
            )}
            placeholder={t("settings.agentQuirksPlaceholder")}
          />
        </div>

        {/* Communication Style */}
        <div>
          <div className="text-xs font-medium text-muted-foreground mb-1 px-1">
            {t("settings.agentCommStyle")}
          </div>
          <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">
            {t("settings.agentCommStyleDesc")}
          </p>
          <Textarea
            className="bg-secondary/40 rounded-lg resize-y leading-relaxed min-h-[60px]"
            rows={3}

            {...textInputProps(config.personality.communicationStyle ?? "", (v) =>
              updatePersonality({ communicationStyle: v || null }),
            )}
            placeholder={t("settings.agentCommStylePlaceholder")}
          />
        </div>

        <div className="border-t border-border/50" />

        {/* Personality supplement */}
        <div>
          <div className="text-xs font-medium text-muted-foreground mb-1 px-1">
            {t("settings.agentSupplement")}
          </div>
          <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">
            {t("settings.agentPersonaSupplementDesc")}
          </p>
          <Textarea
            className="bg-secondary/40 rounded-lg resize-y leading-relaxed font-mono min-h-[120px]"
            rows={8}

            {...textInputProps(persona, setPersona)}
            placeholder={t("settings.agentSupplementPlaceholder")}
          />
          <CharCounter value={persona} />
        </div>
      </div>
    </div>
  )
}
