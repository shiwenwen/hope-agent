import { useState } from "react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { TOOL_I18N_KEY } from "@/types/tools"
import { Switch } from "@/components/ui/switch"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { Button } from "@/components/ui/button"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { ChevronDown } from "lucide-react"
import { OpenClawHintBanner } from "./CustomTab"
import type { AgentConfig, SkillSummary } from "../types"

interface CapabilitiesTabProps {
  config: AgentConfig
  builtinTools: { name: string; description: string; internal?: boolean }[]
  availableSkills: SkillSummary[]
  toolsGuide: string
  openclawMode: boolean
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

export default function CapabilitiesTab({
  config,
  builtinTools,
  availableSkills,
  toolsGuide,
  openclawMode,
  updateConfig,
  setToolsGuide,
  textInputProps,
  CharCounter,
}: CapabilitiesTabProps) {
  const { t } = useTranslation()
  const [toolInjectionOpen, setToolInjectionOpen] = useState(false)

  const toolDisplayName = (name: string) => {
    const key = TOOL_I18N_KEY[name]
    return key ? t(`settings.tool${key}Name`) : name
  }
  const toolDisplayDesc = (name: string) => {
    const key = TOOL_I18N_KEY[name]
    return key ? t(`settings.tool${key}Desc`) : ""
  }

  const injectableTools = builtinTools.filter((tool) => !tool.internal)
  const enabledToolCount = injectableTools.filter(
    (tool) => !config.capabilities.tools.deny.includes(tool.name),
  ).length

  const updateCapabilities = (patch: Partial<AgentConfig["capabilities"]>) =>
    updateConfig({ capabilities: { ...config.capabilities, ...patch } })

  return (
    <Tabs defaultValue="tools" className="w-full">
      <TabsList className="mb-4">
        <TabsTrigger value="tools">{t("settings.agentCapabilitiesTabTools")}</TabsTrigger>
        <TabsTrigger value="skills">{t("settings.agentCapabilitiesTabSkills")}</TabsTrigger>
      </TabsList>

      {/* ─── Tools sub-tab ─────────────────────────────────────── */}
      <TabsContent value="tools" className="space-y-5">
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
              disabled={config.capabilities.maxToolRounds === 0}
              className="flex-1 bg-secondary/40 rounded-lg [appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none [&::-webkit-outer-spin-button]:appearance-none"
              value={
                config.capabilities.maxToolRounds === 0 ? "" : config.capabilities.maxToolRounds
              }
              placeholder={t("settings.agentUnlimited")}
              onChange={(e) => {
                const v = parseInt(e.target.value, 10)
                if (v > 0) updateCapabilities({ maxToolRounds: v })
              }}
            />
            <label className="flex items-center gap-1.5 text-xs text-muted-foreground whitespace-nowrap cursor-pointer select-none">
              <Switch
                checked={config.capabilities.maxToolRounds === 0}
                onCheckedChange={(checked) => updateCapabilities({ maxToolRounds: checked ? 0 : 50 })}
              />
              {t("settings.agentUnlimited")}
            </label>
          </div>
        </div>

        {/* Tool Injection */}
        <div>
          <Button
            type="button"
            variant="ghost"
            className="group h-auto w-full justify-between gap-2 rounded-md px-1 py-1 text-left font-normal hover:bg-transparent"
            onClick={() => setToolInjectionOpen(!toolInjectionOpen)}
          >
            <div className="min-w-0 text-left">
              <div className="text-xs font-medium text-muted-foreground flex items-center gap-1.5">
                {t("settings.agentCapabilitiesToolInjection")}
                {injectableTools.length > 0 && (
                  <span className="text-[11px] text-muted-foreground/50">
                    ({enabledToolCount}/{injectableTools.length})
                  </span>
                )}
              </div>
              <p className="text-[11px] text-muted-foreground/60 mt-0.5">
                {t("settings.agentCapabilitiesToolInjectionDesc")}
              </p>
            </div>
            <ChevronDown
              className={cn(
                "h-4 w-4 text-muted-foreground/50 transition-transform duration-200 shrink-0",
                toolInjectionOpen && "rotate-180",
              )}
            />
          </Button>
          <div
            className={cn(
              "grid transition-[grid-template-rows] duration-200 ease-in-out",
              toolInjectionOpen ? "grid-rows-[1fr]" : "grid-rows-[0fr]",
            )}
          >
            <div className="overflow-hidden">
              {injectableTools.length > 0 && (
                <div className="rounded-lg border border-border/50 overflow-hidden mt-2">
                  {injectableTools.map((tool, idx) => {
                    const isDenied = config.capabilities.tools.deny.includes(tool.name)
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
                          checked={!isDenied}
                          onCheckedChange={(checked) => {
                            const newDeny = checked
                              ? config.capabilities.tools.deny.filter((n) => n !== tool.name)
                              : [...config.capabilities.tools.deny, tool.name]
                            updateCapabilities({
                              tools: { ...config.capabilities.tools, deny: newDeny },
                            })
                          }}
                        />
                      </div>
                    )
                  })}
                </div>
              )}
            </div>
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
          <div className="flex gap-1.5 mb-3">
            {(
              [
                { mode: "all", label: t("settings.agentApprovalAll") },
                { mode: "none", label: t("settings.agentApprovalNone") },
                { mode: "custom", label: t("settings.agentApprovalCustom") },
              ] as const
            ).map(({ mode, label }) => {
              const currentMode = config.capabilities.requireApproval.includes("*")
                ? "all"
                : config.capabilities.requireApproval.length === 0
                  ? "none"
                  : "custom"
              const isActive = currentMode === mode
              return (
                <Button
                  key={mode}
                  variant="outline"
                  size="sm"
                  className={cn(
                    "h-auto rounded-md px-3 py-1.5 text-xs font-normal",
                    isActive
                      ? "bg-primary/10 border-primary/40 text-primary hover:bg-primary/15 hover:text-primary"
                      : "bg-secondary/40 border-border/50 text-muted-foreground hover:border-border",
                  )}
                  onClick={() => {
                    if (mode === "all") {
                      updateCapabilities({ requireApproval: ["*"] })
                    } else if (mode === "none") {
                      updateCapabilities({ requireApproval: [] })
                    } else {
                      updateCapabilities({ requireApproval: ["exec"] })
                    }
                  }}
                >
                  {label}
                </Button>
              )
            })}
          </div>
          {!config.capabilities.requireApproval.includes("*") &&
            config.capabilities.requireApproval.length > 0 && (
              <div className="rounded-lg border border-border/50 overflow-hidden">
                {injectableTools.map((tool, idx) => {
                  const isRequired = config.capabilities.requireApproval.includes(tool.name)
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
                            ? [...config.capabilities.requireApproval, tool.name]
                            : config.capabilities.requireApproval.filter((t) => t !== tool.name)
                          updateCapabilities({
                            requireApproval: newList.length > 0 ? newList : ["exec"],
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
            checked={config.capabilities.sandbox}
            onCheckedChange={(v) => updateCapabilities({ sandbox: v })}
          />
        </div>

        <div className="border-t border-border/50" />

        {/* Tool guidance */}
        <div>
          <div className="text-xs font-medium text-muted-foreground mb-1 px-1">
            {t("settings.agentToolsGuide")}
          </div>
          {openclawMode && (
            <div className="mb-2">
              <OpenClawHintBanner />
            </div>
          )}
          <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">
            {t("settings.agentToolsGuideDesc")}
          </p>
          <Textarea
            className={cn(
              "bg-secondary/40 rounded-lg resize-y leading-relaxed font-mono min-h-[80px]",
              openclawMode && "opacity-60",
            )}
            rows={5}
            readOnly={openclawMode}
            {...(openclawMode
              ? { value: toolsGuide }
              : textInputProps(toolsGuide, setToolsGuide))}
            placeholder={t("settings.agentToolsGuidePlaceholder")}
          />
          <CharCounter value={toolsGuide} />
        </div>
      </TabsContent>

      {/* ─── Skills sub-tab ────────────────────────────────────── */}
      <TabsContent value="skills" className="space-y-5">
        <div>
          <div className="text-xs font-medium text-muted-foreground mb-1 px-1 flex items-center gap-1.5">
            {t("settings.agentSkills")}
            {availableSkills.length > 0 && (
              <span className="text-[11px] text-muted-foreground/50">
                (
                {
                  availableSkills.filter(
                    (s) => !config.capabilities.skills.deny.includes(s.name),
                  ).length
                }
                /{availableSkills.length})
              </span>
            )}
          </div>
          <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">
            {t("settings.agentSkillsDesc")}
          </p>
          {availableSkills.length > 0 && (
            <div className="rounded-lg border border-border/50 overflow-hidden">
              {availableSkills.map((skill, idx) => {
                const isDenied = config.capabilities.skills.deny.includes(skill.name)
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
                          ? config.capabilities.skills.deny.filter((n) => n !== skill.name)
                          : [...config.capabilities.skills.deny, skill.name]
                        updateCapabilities({
                          skills: { ...config.capabilities.skills, deny: newDeny },
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

        <div className="flex items-center justify-between px-1">
          <div>
            <div className="text-sm text-foreground">{t("settings.agentSkillEnvCheck")}</div>
            <div className="text-xs text-muted-foreground">
              {t("settings.agentSkillEnvCheckDesc")}
            </div>
          </div>
          <Switch
            checked={config.capabilities.skillEnvCheck ?? true}
            onCheckedChange={(v) => updateCapabilities({ skillEnvCheck: v })}
          />
        </div>
      </TabsContent>
    </Tabs>
  )
}
