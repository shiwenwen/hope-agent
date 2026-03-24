import { useTranslation } from "react-i18next"
import { Switch } from "@/components/ui/switch"
import { Textarea } from "@/components/ui/textarea"
import type { AgentConfig } from "../types"

interface CustomTabProps {
  config: AgentConfig
  agentMd: string
  persona: string
  updateConfig: (patch: Partial<AgentConfig>) => void
  handleEnableCustomPrompt: () => void
  textInputProps: (getter: string, setter: (v: string) => void) => {
    value: string
    onChange: (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => void
    onCompositionStart: () => void
    onCompositionEnd: (e: React.CompositionEvent<HTMLInputElement | HTMLTextAreaElement>) => void
  }
  setAgentMd: (v: string) => void
  setPersona: (v: string) => void
  CharCounter: React.ComponentType<{ value: string }>
}

export default function CustomTab({
  config,
  agentMd,
  persona,
  updateConfig,
  handleEnableCustomPrompt,
  textInputProps,
  setAgentMd,
  setPersona,
  CharCounter,
}: CustomTabProps) {
  const { t } = useTranslation()

  return (
    <div className="space-y-5">
      {/* Toggle */}
      <div className="flex items-center justify-between px-1">
        <div>
          <div className="text-sm text-foreground">{t("settings.agentCustomPrompt")}</div>
          <div className="text-xs text-muted-foreground">
            {t("settings.agentCustomPromptDesc")}
          </div>
        </div>
        <Switch
          checked={config.useCustomPrompt}
          onCheckedChange={(v) => {
            if (v) handleEnableCustomPrompt()
            else updateConfig({ useCustomPrompt: false })
          }}
        />
      </div>

      {config.useCustomPrompt && (
        <>
          <div className="rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2">
            <p className="text-xs text-amber-600 dark:text-amber-400">
              {t("settings.agentCustomPromptWarning")}
            </p>
          </div>

          {/* Custom Identity */}
          <div>
            <div className="text-xs font-medium text-muted-foreground mb-1 px-1">
              {t("settings.agentMd")}
            </div>
            <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">
              {t("settings.agentCustomIdentityDesc")}
            </p>
            <Textarea
              className="bg-secondary/40 rounded-lg resize-y leading-relaxed font-mono min-h-[160px]"
              rows={10}
              {...textInputProps(agentMd, setAgentMd)}
              placeholder={t("settings.agentMdPlaceholder")}
            />
            <CharCounter value={agentMd} />
          </div>

          {/* Custom Personality */}
          <div>
            <div className="text-xs font-medium text-muted-foreground mb-1 px-1">
              {t("settings.agentPersona")}
            </div>
            <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">
              {t("settings.agentCustomPersonaDesc")}
            </p>
            <Textarea
              className="bg-secondary/40 rounded-lg resize-y leading-relaxed font-mono min-h-[120px]"
              rows={8}
              {...textInputProps(persona, setPersona)}
              placeholder={t("settings.agentPersonaPlaceholder")}
            />
            <CharCounter value={persona} />
          </div>
        </>
      )}
    </div>
  )
}
