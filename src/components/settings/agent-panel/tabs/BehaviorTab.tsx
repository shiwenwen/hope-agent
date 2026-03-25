import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { TOOL_I18N_KEY } from "@/types/tools"
import { Switch } from "@/components/ui/switch"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import type { AgentConfig, SkillSummary } from "../types"

interface BehaviorTabProps {
  config: AgentConfig
  builtinTools: { name: string; description: string; internal?: boolean }[]
  availableSkills: SkillSummary[]
  toolsGuide: string
  updateConfig: (patch: Partial<AgentConfig>) => void
  setToolsGuide: (v: string) => void
  textInputProps: (getter: string, setter: (v: string) => void) => {
    value: string
    onChange: (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => void
    onCompositionStart: () => void
    onCompositionEnd: (e: React.CompositionEvent<HTMLInputElement | HTMLTextAreaElement>) => void
  }
  CharCounter: React.ComponentType<{ value: string }>
}

export default function BehaviorTab({
  config,
  builtinTools,
  availableSkills,
  toolsGuide,
  updateConfig,
  setToolsGuide,
  textInputProps,
  CharCounter,
}: BehaviorTabProps) {
  const { t } = useTranslation()

  const toolDisplayName = (name: string) => {
    const key = TOOL_I18N_KEY[name]
    return key ? t(`settings.tool${key}Name`) : name
  }
  const toolDisplayDesc = (name: string) => {
    const key = TOOL_I18N_KEY[name]
    return key ? t(`settings.tool${key}Desc`) : ""
  }

  return (
    <div className="space-y-5">
      {/* Max Tool Rounds */}
      <div>
        <div className="text-xs font-medium text-muted-foreground mb-2 px-1">
          {t("settings.agentMaxToolRounds")}
        </div>
        <div className="flex items-center gap-3">
          <Input
            type="number"
            min={0}
            max={100}
            disabled={config.behavior.maxToolRounds === 0}
            className="flex-1 bg-secondary/40 rounded-lg [appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none [&::-webkit-outer-spin-button]:appearance-none"
            value={config.behavior.maxToolRounds === 0 ? "" : config.behavior.maxToolRounds}
            placeholder={t("settings.agentUnlimited")}
            onChange={(e) => {
              const v = parseInt(e.target.value, 10)
              if (v > 0)
                updateConfig({ behavior: { ...config.behavior, maxToolRounds: v } })
            }}
          />
          <label className="flex items-center gap-1.5 text-xs text-muted-foreground whitespace-nowrap cursor-pointer select-none">
            <input
              type="checkbox"
              className="rounded"
              checked={config.behavior.maxToolRounds === 0}
              onChange={(e) => {
                updateConfig({
                  behavior: {
                    ...config.behavior,
                    maxToolRounds: e.target.checked ? 0 : 10,
                  },
                })
              }}
            />
            {t("settings.agentUnlimited")}
          </label>
        </div>
      </div>

      {/* Require Approval */}
      <div>
        <div className="text-xs font-medium text-muted-foreground mb-1 px-1">
          {t("settings.agentRequireApproval")}
        </div>
        <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">
          {t("settings.agentRequireApprovalDesc")}
        </p>
        {/* Mode selector */}
        <div className="flex gap-1.5 mb-3">
          {(
            [
              { mode: "all", label: t("settings.agentApprovalAll") },
              { mode: "none", label: t("settings.agentApprovalNone") },
              { mode: "custom", label: t("settings.agentApprovalCustom") },
            ] as const
          ).map(({ mode, label }) => {
            const currentMode = config.behavior.requireApproval.includes("*")
              ? "all"
              : config.behavior.requireApproval.length === 0
                ? "none"
                : "custom"
            const isActive = currentMode === mode
            return (
              <button
                key={mode}
                className={cn(
                  "px-3 py-1.5 text-xs rounded-md border transition-colors",
                  isActive
                    ? "bg-primary/10 border-primary/40 text-primary"
                    : "bg-secondary/40 border-border/50 text-muted-foreground hover:border-border",
                )}
                onClick={() => {
                  if (mode === "all") {
                    updateConfig({
                      behavior: { ...config.behavior, requireApproval: ["*"] },
                    })
                  } else if (mode === "none") {
                    updateConfig({ behavior: { ...config.behavior, requireApproval: [] } })
                  } else {
                    updateConfig({
                      behavior: { ...config.behavior, requireApproval: ["exec"] },
                    })
                  }
                }}
              >
                {label}
              </button>
            )
          })}
        </div>
        {/* Custom tool selection */}
        {!config.behavior.requireApproval.includes("*") &&
          config.behavior.requireApproval.length > 0 && (
            <div className="rounded-lg border border-border/50 overflow-hidden">
              {builtinTools.filter((t) => !t.internal).map((tool, idx) => {
                const isRequired = config.behavior.requireApproval.includes(tool.name)
                return (
                  <div
                    key={tool.name}
                    className={cn(
                      "flex items-center justify-between px-3 py-2 gap-3",
                      idx > 0 && "border-t border-border/30",
                    )}
                  >
                    <div className="min-w-0 flex-1">
                      <div className="text-xs font-medium text-foreground">
                        {toolDisplayName(tool.name)}
                      </div>
                      <div className="text-[11px] text-muted-foreground/60 line-clamp-1">
                        {toolDisplayDesc(tool.name)}
                      </div>
                    </div>
                    <Switch
                      checked={isRequired}
                      onCheckedChange={(checked) => {
                        const newList = checked
                          ? [...config.behavior.requireApproval, tool.name]
                          : config.behavior.requireApproval.filter((t) => t !== tool.name)
                        updateConfig({
                          behavior: {
                            ...config.behavior,
                            requireApproval: newList.length > 0 ? newList : ["exec"],
                          },
                        })
                      }}
                    />
                  </div>
                )
              })}
            </div>
          )}
      </div>

      <div className="border-t border-border/50" />

      {/* Sandbox */}
      <div className="flex items-center justify-between px-1">
        <div>
          <div className="text-sm text-foreground">{t("settings.agentSandbox")}</div>
          <div className="text-xs text-muted-foreground">
            {t("settings.agentSandboxDesc")}
          </div>
        </div>
        <Switch
          checked={config.behavior.sandbox}
          onCheckedChange={(v) =>
            updateConfig({ behavior: { ...config.behavior, sandbox: v } })
          }
        />
      </div>

      <div className="border-t border-border/50" />

      {/* Skills */}
      <div>
        <div className="text-xs font-medium text-muted-foreground mb-1 px-1">
          {t("settings.agentSkills")}
        </div>
        <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">
          {t("settings.agentSkillsDesc")}
        </p>
        {availableSkills.length > 0 && (
          <div className="rounded-lg border border-border/50 overflow-hidden mb-3">
            {availableSkills.map((skill, idx) => {
              const isDenied = config.skills.deny.includes(skill.name)
              return (
                <div
                  key={skill.name}
                  className={cn(
                    "flex items-center justify-between px-3 py-2 gap-3",
                    idx > 0 && "border-t border-border/30",
                  )}
                >
                  <div className="min-w-0 flex-1">
                    <div className="text-xs font-medium text-foreground truncate">
                      {skill.name}
                    </div>
                    <div className="text-[11px] text-muted-foreground/60 truncate">
                      {skill.description}
                    </div>
                  </div>
                  <Switch
                    checked={!isDenied}
                    onCheckedChange={(checked) => {
                      const newDeny = checked
                        ? config.skills.deny.filter((n) => n !== skill.name)
                        : [...config.skills.deny, skill.name]
                      updateConfig({ skills: { ...config.skills, deny: newDeny } })
                    }}
                  />
                </div>
              )
            })}
          </div>
        )}
        {/* Skill env check */}
        <div className="flex items-center justify-between px-1">
          <div>
            <div className="text-sm text-foreground">
              {t("settings.agentSkillEnvCheck")}
            </div>
            <div className="text-xs text-muted-foreground">
              {t("settings.agentSkillEnvCheckDesc")}
            </div>
          </div>
          <Switch
            checked={config.behavior.skillEnvCheck ?? true}
            onCheckedChange={(v) =>
              updateConfig({ behavior: { ...config.behavior, skillEnvCheck: v } })
            }
          />
        </div>
      </div>

      <div className="border-t border-border/50" />

      {/* Tool guidance */}
      <div>
        <div className="text-xs font-medium text-muted-foreground mb-1 px-1">
          {t("settings.agentToolsGuide")}
        </div>
        <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">
          {t("settings.agentToolsGuideDesc")}
        </p>
        <Textarea
          className="bg-secondary/40 rounded-lg resize-y leading-relaxed font-mono min-h-[80px]"
          rows={5}
          {...textInputProps(toolsGuide, setToolsGuide)}
          placeholder={t("settings.agentToolsGuidePlaceholder")}
        />
        <CharCounter value={toolsGuide} />
      </div>
    </div>
  )
}
