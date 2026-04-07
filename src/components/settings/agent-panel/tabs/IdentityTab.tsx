import { useTranslation } from "react-i18next"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { OpenClawHintBanner } from "./CustomTab"
import type { AgentConfig, PersonalityConfig } from "../types"

interface IdentityTabProps {
  config: AgentConfig
  agentMd: string
  openclawMode: boolean
  updateConfig: (patch: Partial<AgentConfig>) => void
  updatePersonality: (patch: Partial<PersonalityConfig>) => void
  setAgentMd: (v: string) => void
  textInputProps: (getter: string, setter: (v: string) => void) => {
    value: string
    onChange: (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => void
    onCompositionStart: () => void
    onCompositionEnd: (e: React.CompositionEvent<HTMLInputElement | HTMLTextAreaElement>) => void
  }
  CharCounter: React.ComponentType<{ value: string }>
}

export default function IdentityTab({
  config,
  agentMd,
  openclawMode,
  updateConfig,
  updatePersonality,
  setAgentMd,
  textInputProps,
  CharCounter,
}: IdentityTabProps) {
  const { t } = useTranslation()

  return (
    <div className="space-y-4">
      {openclawMode && <OpenClawHintBanner />}

      {/* Name */}
      <div>
        <div className="text-xs font-medium text-muted-foreground mb-2 px-1">
          {t("settings.agentName")}
        </div>
        <Input
          className="bg-secondary/40 rounded-lg"
          {...textInputProps(config.name, (v) => updateConfig({ name: v }))}
          placeholder={t("settings.agentNamePlaceholder")}
        />
      </div>

      {/* Description */}
      <div>
        <div className="text-xs font-medium text-muted-foreground mb-2 px-1">
          {t("settings.agentDescription")}
        </div>
        <Textarea
          className="bg-secondary/40 rounded-lg resize-y leading-relaxed min-h-[50px]"
          rows={2}
          {...textInputProps(config.description ?? "", (v) =>
            updateConfig({ description: v || null }),
          )}
          placeholder={t("settings.agentDescriptionPlaceholder")}
        />
      </div>

      {/* Emoji */}
      <div>
        <div className="text-xs font-medium text-muted-foreground mb-2 px-1">
          {t("settings.agentEmoji")}
        </div>
        <Input
          className="bg-secondary/40 rounded-lg"
          {...textInputProps(config.emoji ?? "", (v) => updateConfig({ emoji: v || null }))}
          placeholder={t("settings.agentEmojiPlaceholder")}
        />
      </div>

      <div className="border-t border-border/50" />

      {/* Role */}
      <div className={openclawMode ? "opacity-50 pointer-events-none" : ""}>
        <div className="text-xs font-medium text-muted-foreground mb-2 px-1">
          {t("settings.agentRole")}
        </div>
        <Textarea
          className="bg-secondary/40 rounded-lg resize-y leading-relaxed min-h-[60px]"
          rows={3}
          disabled={openclawMode}
          {...textInputProps(config.personality.role ?? "", (v) =>
            updatePersonality({ role: v || null }),
          )}
          placeholder={t("settings.agentRolePlaceholder")}
        />
      </div>

      <div className="border-t border-border/50" />

      {/* Identity supplement */}
      <div className={openclawMode ? "opacity-50 pointer-events-none" : ""}>
        <div className="text-xs font-medium text-muted-foreground mb-1 px-1">
          {t("settings.agentSupplement")}
        </div>
        <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">
          {t("settings.agentIdentitySupplementDesc")}
        </p>
        <Textarea
          className="bg-secondary/40 rounded-lg resize-y leading-relaxed font-mono min-h-[120px]"
          rows={8}
          disabled={openclawMode}
          {...textInputProps(agentMd, setAgentMd)}
          placeholder={t("settings.agentSupplementPlaceholder")}
        />
        <CharCounter value={agentMd} />
      </div>
    </div>
  )
}
