import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { isMainAgent, TOOL_I18N_KEY, TOOL_NAME_TO_TOGGLE_KEY } from "@/types/tools"
import { Switch } from "@/components/ui/switch"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { Button } from "@/components/ui/button"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { OpenClawHintBanner } from "./CustomTab"
import type { AgentConfig, SkillSummary } from "../types"

/** Tier metadata returned by the backend `list_builtin_tools` command. */
type BuiltinTool = {
  name: string
  description: string
  internal?: boolean
  tier?: "core" | "standard" | "configured" | "memory" | "mcp"
  core_subclass?: string | null
  default_for_main?: boolean | null
  default_for_others?: boolean | null
  config_hint?: string | null
}

interface CapabilitiesTabProps {
  config: AgentConfig
  agentId: string
  builtinTools: BuiltinTool[]
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
  agentId,
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
  const isMain = isMainAgent(agentId)

  const toolDisplayName = (name: string) => {
    const key = TOOL_I18N_KEY[name]
    return key ? t(`settings.tool${key}Name`) : name
  }
  const toolDisplayDesc = (name: string) => {
    const key = TOOL_I18N_KEY[name]
    return key ? t(`settings.tool${key}Desc`) : ""
  }

  const updateCapabilities = (patch: Partial<AgentConfig["capabilities"]>) =>
    updateConfig({ capabilities: { ...config.capabilities, ...patch } })

  // ── Tier grouping ───────────────────────────────────────────────
  // Buckets each tool into its tier section. Core::PlanMode / Core::Meta
  // are framework-only — they're always-on and aren't surfaced to the user.
  const coreVisibleTools = builtinTools.filter(
    (t) =>
      t.tier === "core" &&
      t.core_subclass !== "plan_mode" &&
      t.core_subclass !== "meta",
  )
  const standardTools = builtinTools.filter((t) => t.tier === "standard")
  const configuredTools = builtinTools.filter((t) => t.tier === "configured")
  const approvalTools = builtinTools.filter((t) => t.internal !== true)
  // memory / mcp tools are not individually toggleable in this UI — they're
  // controlled by the global memory enabled flag and the MCP master switch.

  // ── Helpers for Tier 2 (Standard, controlled via tools.deny) ──
  const tier2Enabled = (tool: BuiltinTool) => {
    if (config.capabilities.tools.deny.includes(tool.name)) return false
    if (config.capabilities.tools.allow.includes(tool.name)) return true
    return isMain ? !!tool.default_for_main : !!tool.default_for_others
  }
  const setTier2Enabled = (tool: BuiltinTool, on: boolean) => {
    const tierDefault = isMain
      ? !!tool.default_for_main
      : !!tool.default_for_others
    const inDeny = config.capabilities.tools.deny.includes(tool.name)
    let newDeny = config.capabilities.tools.deny.slice()
    if (on) {
      // Want it on. If denied, remove from deny. If tier default is off and we
      // want it on, we still need to remove from deny (deny absent = use default,
      // but default is off — so we'd need an "allow" list. Tier 2 doesn't
      // currently support enabling-via-allow because tools.allow is unused.
      // Practical effect: turning on a tool whose tier default is off becomes
      // a no-op unless we add the override to tools.allow. To keep semantics
      // honest, we treat allow as the override list for Tier 2.)
      newDeny = newDeny.filter((n) => n !== tool.name)
      const allow = config.capabilities.tools.allow.slice()
      if (!tierDefault && !allow.includes(tool.name)) allow.push(tool.name)
      updateCapabilities({
        tools: { allow, deny: newDeny },
      })
    } else {
      if (!inDeny) newDeny.push(tool.name)
      const allow = config.capabilities.tools.allow.filter((n) => n !== tool.name)
      updateCapabilities({
        tools: { allow, deny: newDeny },
      })
    }
  }

  // ── Helpers for Tier 3 (Configured, controlled via capabilityToggles) ──
  const tier3Resolved = (tool: BuiltinTool): boolean => {
    const key = TOOL_NAME_TO_TOGGLE_KEY[tool.name]
    if (!key) return false
    const explicit = config.capabilities.capabilityToggles?.[key]
    if (explicit === true || explicit === false) return explicit
    return isMain ? !!tool.default_for_main : !!tool.default_for_others
  }
  const setTier3Toggle = (tool: BuiltinTool, on: boolean) => {
    const key = TOOL_NAME_TO_TOGGLE_KEY[tool.name]
    if (!key) return
    const next = { ...(config.capabilities.capabilityToggles ?? {}), [key]: on }
    updateCapabilities({ capabilityToggles: next })
  }

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
                onCheckedChange={(checked) =>
                  updateCapabilities({ maxToolRounds: checked ? 0 : 50 })
                }
              />
              {t("settings.agentUnlimited")}
            </label>
          </div>
        </div>

        {/* Tier 1: Core (read-only listing) */}
        {coreVisibleTools.length > 0 && (
          <div>
            <div className="text-xs font-medium text-muted-foreground px-1">
              {t("settings.agentTierCoreTitle")}
            </div>
            <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">
              {t("settings.agentTierCoreDesc")}
            </p>
            <div className="rounded-lg border border-border/50 overflow-hidden bg-secondary/20">
              {coreVisibleTools.map((tool, idx) => (
                <div
                  key={tool.name}
                  className={cn(
                    "flex items-center justify-between px-3 py-2 gap-3 opacity-70",
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
                  <span className="text-[10px] text-muted-foreground/60 px-2 py-0.5 rounded bg-secondary/60">
                    {t("settings.agentTierCoreBadge")}
                  </span>
                </div>
              ))}
            </div>
          </div>
        )}

        {/* Tier 2: Standard */}
        {standardTools.length > 0 && (
          <div>
            <div className="text-xs font-medium text-muted-foreground px-1">
              {t("settings.agentTierStandardTitle")}
            </div>
            <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">
              {t("settings.agentTierStandardDesc")}
            </p>
            <div className="rounded-lg border border-border/50 overflow-hidden">
              {standardTools.map((tool, idx) => (
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
                    checked={tier2Enabled(tool)}
                    onCheckedChange={(checked) => setTier2Enabled(tool, checked)}
                  />
                </div>
              ))}
            </div>
          </div>
        )}

        {/* Tier 3: Configured */}
        {configuredTools.length > 0 && (
          <div>
            <div className="text-xs font-medium text-muted-foreground px-1">
              {t("settings.agentTierConfiguredTitle")}
            </div>
            <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">
              {t("settings.agentTierConfiguredDesc")}
            </p>
            <div className="rounded-lg border border-border/50 overflow-hidden">
              {configuredTools.map((tool, idx) => {
                const enabled = tier3Resolved(tool)
                return (
                  <div
                    key={tool.name}
                    className={cn(
                      "flex flex-col px-3 py-2 gap-1",
                      idx > 0 && "border-t border-border/30",
                    )}
                  >
                    <div className="flex items-center justify-between gap-3">
                      <div className="min-w-0 flex-1">
                        <div className="text-xs font-medium text-foreground">
                          {toolDisplayName(tool.name)}
                        </div>
                        <div className="text-[11px] text-muted-foreground/60 line-clamp-1">
                          {toolDisplayDesc(tool.name)}
                        </div>
                      </div>
                      <Switch
                        checked={enabled}
                        onCheckedChange={(checked) => setTier3Toggle(tool, checked)}
                      />
                    </div>
                    {enabled && tool.config_hint && (
                      <div className="text-[10px] text-amber-500/80 dark:text-amber-400/80 mt-0.5">
                        {t("settings.agentTierConfiguredHint", { hint: tool.config_hint })}
                      </div>
                    )}
                  </div>
                )
              })}
            </div>
          </div>
        )}

        {/* MCP master switch */}
        <div>
          <div className="text-xs font-medium text-muted-foreground px-1">
            {t("settings.agentMcpTitle")}
          </div>
          <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">
            {t("settings.agentMcpDesc")}
          </p>
          <div className="flex items-center justify-between px-3 py-2 rounded-lg border border-border/50">
            <div className="min-w-0 flex-1">
              <div className="text-xs font-medium text-foreground">
                {t("settings.agentMcpEnableLabel")}
              </div>
            </div>
            <Switch
              checked={config.capabilities.mcpEnabled ?? true}
              onCheckedChange={(v) => updateCapabilities({ mcpEnabled: v })}
            />
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
                {approvalTools.map((tool, idx) => {
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
