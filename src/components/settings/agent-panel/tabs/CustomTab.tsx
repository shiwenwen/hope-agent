import { useState } from "react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { Switch } from "@/components/ui/switch"
import { Textarea } from "@/components/ui/textarea"
import { Button } from "@/components/ui/button"
import type { AgentConfig } from "../types"

/** Reusable hint banner for OpenClaw-disabled tabs */
export function OpenClawHintBanner() {
  const { t } = useTranslation()
  return (
    <div className="rounded-lg border border-blue-500/30 bg-blue-500/5 px-3 py-2">
      <p className="text-xs text-blue-600 dark:text-blue-400">
        {t("settings.openclawModeActiveHint")}
      </p>
    </div>
  )
}

interface CustomTabProps {
  config: AgentConfig
  agentsMd: string
  identityMd: string
  soulMd: string
  toolsGuide: string
  updateConfig: (patch: Partial<AgentConfig>) => void
  handleEnableOpenClawMode: () => void
  textInputProps: (getter: string, setter: (v: string) => void) => {
    value: string
    onChange: (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => void
    onCompositionStart: () => void
    onCompositionEnd: (e: React.CompositionEvent<HTMLInputElement | HTMLTextAreaElement>) => void
  }
  setAgentsMd: (v: string) => void
  setIdentityMd: (v: string) => void
  setSoulMd: (v: string) => void
  setToolsGuide: (v: string) => void
  CharCounter: React.ComponentType<{ value: string }>
}

type OcFileTab = "agents" | "identity" | "soul" | "tools"

const OC_FILE_TABS: { id: OcFileTab; labelKey: string; descKey: string; placeholder: string }[] = [
  { id: "agents", labelKey: "settings.agentAgentsMd", descKey: "settings.agentAgentsMdDesc", placeholder: "# AGENTS.md - Your Workspace" },
  { id: "identity", labelKey: "settings.agentIdentityMd", descKey: "settings.agentIdentityMdDesc", placeholder: "# IDENTITY.md - Who Am I?" },
  { id: "soul", labelKey: "settings.agentSoulMd", descKey: "settings.agentSoulMdDesc", placeholder: "# SOUL.md - Who You Are" },
  { id: "tools", labelKey: "settings.agentToolsMd", descKey: "settings.agentToolsMdOcDesc", placeholder: "# TOOLS.md - Local Notes" },
]

export default function CustomTab({
  config,
  agentsMd,
  identityMd,
  soulMd,
  toolsGuide,
  updateConfig,
  handleEnableOpenClawMode,
  textInputProps,
  setAgentsMd,
  setIdentityMd,
  setSoulMd,
  setToolsGuide,
  CharCounter,
}: CustomTabProps) {
  const { t } = useTranslation()
  const [activeFile, setActiveFile] = useState<OcFileTab>("agents")

  const fileState: Record<OcFileTab, { value: string; setter: (v: string) => void }> = {
    agents: { value: agentsMd, setter: setAgentsMd },
    identity: { value: identityMd, setter: setIdentityMd },
    soul: { value: soulMd, setter: setSoulMd },
    tools: { value: toolsGuide, setter: setToolsGuide },
  }

  const activeTab = OC_FILE_TABS.find((t) => t.id === activeFile)!
  const { value, setter } = fileState[activeFile]

  return (
    <div className="space-y-5">
      {/* OpenClaw Compatible Mode Toggle */}
      <div className="flex items-center justify-between px-1">
        <div>
          <div className="text-sm text-foreground">{t("settings.agentOpenClawMode")}</div>
          <div className="text-xs text-muted-foreground">
            {t("settings.agentOpenClawModeDesc")}
          </div>
        </div>
        <Switch
          checked={config.openclawMode}
          onCheckedChange={(v) => {
            if (v) handleEnableOpenClawMode()
            else updateConfig({ openclawMode: false })
          }}
        />
      </div>

      {config.openclawMode && (
        <>
          <div className="rounded-lg border border-blue-500/30 bg-blue-500/5 px-3 py-2 space-y-1">
            <p className="text-xs text-blue-600 dark:text-blue-400">
              {t("settings.agentOpenClawModeWarning")}
            </p>
            <p className="text-xs text-blue-600 dark:text-blue-400">
              {t("settings.openclawMemoryMdHint")}
            </p>
          </div>

          {/* Sub-tabs for 4 md files */}
          <div className="flex gap-1 border-b border-border pb-px">
            {OC_FILE_TABS.map((tab) => (
              <Button
                key={tab.id}
                variant="ghost"
                size="sm"
                className={cn(
                  "h-auto rounded-t-md rounded-b-none px-2.5 py-1.5 text-xs font-normal -mb-px",
                  activeFile === tab.id
                    ? "text-primary border-b-2 border-primary font-medium hover:bg-transparent hover:text-primary"
                    : "text-muted-foreground hover:bg-transparent hover:text-foreground",
                )}
                onClick={() => setActiveFile(tab.id)}
              >
                {t(tab.labelKey)}
              </Button>
            ))}
          </div>

          {/* Active file editor */}
          <div>
            <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">
              {t(activeTab.descKey)}
            </p>
            <Textarea
              className="bg-secondary/40 rounded-lg resize-y leading-relaxed font-mono min-h-[280px]"
              rows={16}
              {...textInputProps(value, setter)}
              placeholder={activeTab.placeholder}
            />
            <CharCounter value={value} />
          </div>
        </>
      )}
    </div>
  )
}
