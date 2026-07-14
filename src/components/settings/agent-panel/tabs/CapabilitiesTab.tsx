import { useCallback, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { ChevronDown, ChevronRight } from "lucide-react"
import { cn } from "@/lib/utils"
import {
  isMainAgent,
  toolDisplayDescFallback,
  toolDisplayNameFallback,
  TOOL_I18N_KEY,
} from "@/types/tools"
import { Switch } from "@/components/ui/switch"
import { NumberInput } from "@/components/ui/number-input"
import { Textarea } from "@/components/ui/textarea"
import { Button } from "@/components/ui/button"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { OpenClawHintBanner } from "./CustomTab"
import { DockerSetupHint } from "../../DockerSetupHint"
import type { DockerStatus } from "../../dockerSetup"
import type { AgentConfig, AsyncToolPolicy, SkillSummary } from "../types"
import type { SandboxMode } from "@/types/chat"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"

/** Ordered policy options. First entry is the implicit default. The
 * `i18nKey` segment plugs into `settings.agentAsyncToolPolicy.<key>` —
 * see [`src/i18n/locales/en.json`](../../../i18n/locales/en.json) for
 * the mirrored nested structure. */
const ASYNC_TOOL_POLICIES: ReadonlyArray<{ value: AsyncToolPolicy; i18nKey: string }> = [
  { value: "model-decide", i18nKey: "modelDecide" },
  { value: "always-background", i18nKey: "alwaysBackground" },
  { value: "never-background", i18nKey: "neverBackground" },
]
const ASYNC_TOOL_POLICY_DEFAULT = ASYNC_TOOL_POLICIES[0].value
const asyncToolPolicyI18nKey = (value: AsyncToolPolicy) =>
  ASYNC_TOOL_POLICIES.find((policy) => policy.value === value)?.i18nKey ??
  ASYNC_TOOL_POLICIES[0].i18nKey

const SANDBOX_MODES: ReadonlyArray<SandboxMode> = [
  "off",
  "standard",
  "isolated",
  "workspace",
  "trusted",
]

function sandboxModeDescription(mode: SandboxMode): string {
  switch (mode) {
    case "off":
      return "在宿主机执行，审批逻辑不变"
    case "standard":
      return "在 Docker 沙箱执行，审批不放松"
    case "isolated":
      return "隔离副本试跑，编辑审批不放松"
    case "workspace":
      return "挂载当前工作区，减少编辑命令审批"
    case "trusted":
      return "沙箱内 exec 最大自治，严格风险仍审批"
  }
}

/** Collapsible section wrapper used by every tier block in this tab. */
function CollapsibleSection({
  title,
  description,
  badge,
  open,
  onToggle,
  children,
}: {
  title: string
  description?: string
  badge?: React.ReactNode
  open: boolean
  onToggle: () => void
  children: React.ReactNode
}) {
  return (
    <div>
      <Button
        variant="ghost"
        type="button"
        className="h-auto w-full justify-start gap-2 rounded-md px-1 py-1 text-left font-normal hover:bg-secondary/40"
        onClick={onToggle}
      >
        {open ? (
          <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        ) : (
          <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        )}
        <span className="text-xs font-medium text-muted-foreground">{title}</span>
        {badge != null && (
          <span className="text-[11px] text-muted-foreground/50 ml-1">{badge}</span>
        )}
      </Button>
      {open && (
        <div className="mt-2">
          {description && (
            <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">{description}</p>
          )}
          {children}
        </div>
      )}
    </div>
  )
}

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
  /** Tier 3 only — `null` for other tiers. `false` means provider/feature
   * not yet configured globally; the UI should surface the hint banner. */
  globally_configured?: boolean | null
  defer_capable?: boolean | null
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
  const { t, i18n } = useTranslation()
  const isMain = isMainAgent(agentId)

  // ── Collapsible state (default collapsed to keep the tab compact) ──
  const [coreOpen, setCoreOpen] = useState(false)
  const [standardOpen, setStandardOpen] = useState(false)
  const [mcpOpen, setMcpOpen] = useState(false)
  const [skillsOpen, setSkillsOpen] = useState(false)
  const [dockerStatus, setDockerStatus] = useState<DockerStatus | null>(null)
  const [dockerChecking, setDockerChecking] = useState(false)
  const asyncToolPolicyValue =
    config.capabilities.asyncToolPolicy ?? ASYNC_TOOL_POLICY_DEFAULT
  const sandboxMode: SandboxMode =
    config.capabilities.defaultSandboxMode ??
    (config.capabilities.sandbox ? "standard" : "off")

  const refreshDockerStatus = useCallback(async () => {
    setDockerChecking(true)
    try {
      const status = await getTransport().call<DockerStatus>("check_sandbox_available")
      setDockerStatus(status)
    } catch (e) {
      logger.error("settings", "CapabilitiesTab", "Failed to check Docker status", e)
    } finally {
      setDockerChecking(false)
    }
  }, [])

  useEffect(() => {
    if (sandboxMode === "off") return
    void refreshDockerStatus()
  }, [refreshDockerStatus, sandboxMode])

  const toolDisplayName = (tool: BuiltinTool) => {
    const name = tool.name
    const key = TOOL_I18N_KEY[name]
    return key ? t(`settings.tool${key}Name`) : toolDisplayNameFallback(name, i18n.language)
  }
  const toolDisplayDesc = (tool: BuiltinTool) => {
    const name = tool.name
    const key = TOOL_I18N_KEY[name]
    return key
      ? t(`settings.tool${key}Desc`)
      : toolDisplayDescFallback(name, tool.description, i18n.language)
  }

  const updateCapabilities = (patch: Partial<AgentConfig["capabilities"]>) =>
    updateConfig({ capabilities: { ...config.capabilities, ...patch } })

  const updateSandboxMode = (mode: SandboxMode) =>
    updateCapabilities({
      defaultSandboxMode: mode,
      sandbox: mode !== "off",
    })

  // ── Tier grouping ───────────────────────────────────────────────
  // Buckets each tool into its tier section. Core::PlanMode / Core::Meta
  // are framework-only — they're always-on and aren't surfaced to the user.
  const coreVisibleTools = builtinTools.filter(
    (t) =>
      t.tier === "core" &&
      t.core_subclass !== "plan_mode" &&
      t.core_subclass !== "meta",
  )
  // Tier 2 (Standard) and Tier 3 (Configured) share the same UI section.
  // Individual switches write `capabilities.tools.allow/deny`; Tier 3 still
  // renders an extra "needs global provider/account" hint when enabled but
  // not globally provisioned.
  const userToggleableTools = builtinTools.filter(
    (t) => t.tier === "standard" || t.tier === "configured",
  )
  // memory / mcp tools are not individually toggleable in this UI — they're
  // controlled by the global memory enabled flag and the MCP master switch.

  // ── Helpers for user-toggleable tool defaults + explicit overrides ──
  const toolDefaultEnabled = (tool: BuiltinTool) =>
    isMain ? !!tool.default_for_main : !!tool.default_for_others

  const toolEnabled = (tool: BuiltinTool) => {
    if (config.capabilities.tools.deny.includes(tool.name)) return false
    if (config.capabilities.tools.allow.includes(tool.name)) return true
    return toolDefaultEnabled(tool)
  }

  const setToolEnabled = (tool: BuiltinTool, on: boolean) => {
    const allow = config.capabilities.tools.allow.filter((n) => n !== tool.name)
    const deny = config.capabilities.tools.deny.filter((n) => n !== tool.name)
    const defaultEnabled = toolDefaultEnabled(tool)
    const tools = {
      allow: on && !defaultEnabled ? [...allow, tool.name] : allow,
      deny: !on && defaultEnabled ? [...deny, tool.name] : deny,
    }
    updateCapabilities({ tools })
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
            <NumberInput
              min={0}
              max={100}
              disabled={config.capabilities.maxToolRounds === 0}
              className="flex-1"
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

        {/* Async tool backgrounding policy. Title-only — each option carries
            its own description inside the dropdown, so no leading <p> here. */}
        <div>
          <div className="text-xs font-medium text-muted-foreground mb-2 px-1">
            {t("settings.agentAsyncToolPolicy.title")}
          </div>
          <Select
            value={asyncToolPolicyValue}
            onValueChange={(v) =>
              updateCapabilities({ asyncToolPolicy: v as AsyncToolPolicy })
            }
          >
            <SelectTrigger className="h-8 w-full text-sm">
              <SelectValue>
                {t(
                  `settings.agentAsyncToolPolicy.${asyncToolPolicyI18nKey(
                    asyncToolPolicyValue,
                  )}.label`,
                )}
              </SelectValue>
            </SelectTrigger>
            <SelectContent>
              {ASYNC_TOOL_POLICIES.map(({ value, i18nKey }) => (
                <SelectItem key={value} value={value}>
                  <div className="flex flex-col gap-0.5 text-left">
                    <span>{t(`settings.agentAsyncToolPolicy.${i18nKey}.label`)}</span>
                    <span className="text-[11px] text-muted-foreground/70">
                      {t(`settings.agentAsyncToolPolicy.${i18nKey}.desc`)}
                    </span>
                  </div>
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        {/* Tier 1: Core (read-only listing) */}
        {coreVisibleTools.length > 0 && (
          <CollapsibleSection
            title={t("settings.agentTierCoreTitle")}
            description={t("settings.agentTierCoreDesc")}
            badge={`(${coreVisibleTools.length})`}
            open={coreOpen}
            onToggle={() => setCoreOpen((v) => !v)}
          >
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
                      {toolDisplayName(tool)}
                    </div>
                    <div className="text-[11px] text-muted-foreground/60 line-clamp-1">
                      {toolDisplayDesc(tool)}
                    </div>
                  </div>
                  <span className="text-[10px] text-muted-foreground/60 px-2 py-0.5 rounded bg-secondary/60">
                    {t("settings.agentTierCoreBadge")}
                  </span>
                </div>
              ))}
            </div>
          </CollapsibleSection>
        )}

        {/* Standard tools (Tier 2 + Tier 3 merged) */}
        {userToggleableTools.length > 0 && (
          <CollapsibleSection
            title={t("settings.agentTierStandardTitle")}
            description={t("settings.agentTierStandardDesc")}
            badge={(() => {
              const enabled = userToggleableTools.filter(toolEnabled).length
              return `(${enabled}/${userToggleableTools.length})`
            })()}
            open={standardOpen}
            onToggle={() => setStandardOpen((v) => !v)}
          >
            <div className="rounded-lg border border-border/50 overflow-hidden">
              {userToggleableTools.map((tool, idx) => {
                const enabled = toolEnabled(tool)
                const showHint =
                  enabled &&
                  tool.tier === "configured" &&
                  !!tool.config_hint &&
                  tool.globally_configured === false
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
                          {toolDisplayName(tool)}
                        </div>
                        <div className="text-[11px] text-muted-foreground/60 line-clamp-1">
                          {toolDisplayDesc(tool)}
                        </div>
                      </div>
                      <Switch
                        checked={enabled}
                        onCheckedChange={(checked) => setToolEnabled(tool, checked)}
                      />
                    </div>
                    {showHint && (
                      <div className="text-[10px] text-amber-500/80 dark:text-amber-400/80 mt-0.5">
                        {t("settings.agentTierConfiguredHint", { hint: tool.config_hint })}
                      </div>
                    )}
                  </div>
                )
              })}
            </div>
          </CollapsibleSection>
        )}

        {/* MCP master switch */}
        <CollapsibleSection
          title={t("settings.agentMcpTitle")}
          description={t("settings.agentMcpDesc")}
          badge={(config.capabilities.mcpEnabled ?? true) ? t("settings.agentMcpBadgeOn") : t("settings.agentMcpBadgeOff")}
          open={mcpOpen}
          onToggle={() => setMcpOpen((v) => !v)}
        >
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
        </CollapsibleSection>

        {/* Sandbox */}
        <div className="space-y-2 px-1">
          <div>
            <div className="text-sm text-foreground">{t("settings.agentSandbox")}</div>
            <div className="text-xs text-muted-foreground">
              {t("settings.agentSandboxDesc")}
            </div>
          </div>
          <Select value={sandboxMode} onValueChange={(v) => updateSandboxMode(v as SandboxMode)}>
            <SelectTrigger className="h-8 w-full text-sm">
              <SelectValue>
                {t(`chat.sandboxMode.${sandboxMode}.label`, {
                  defaultValue: sandboxMode,
                })}
              </SelectValue>
            </SelectTrigger>
            <SelectContent>
              {SANDBOX_MODES.map((mode) => (
                <SelectItem key={mode} value={mode}>
                  <div className="flex flex-col py-1">
                    <span>
                      {t(`chat.sandboxMode.${mode}.label`, { defaultValue: mode })}
                    </span>
                    <span className="text-xs text-muted-foreground">
                      {t(`chat.sandboxMode.${mode}.desc`, {
                        defaultValue: sandboxModeDescription(mode),
                      })}
                    </span>
                  </div>
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {sandboxMode !== "off" && (
            <DockerSetupHint
              status={dockerStatus}
              checking={dockerChecking}
              onRefresh={refreshDockerStatus}
              title={t("chat.sandboxMode.setupTitle", {
                defaultValue: "配置 Docker 后启用沙箱",
              })}
            />
          )}
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
        <CollapsibleSection
          title={t("settings.agentSkills")}
          description={t("settings.agentSkillsDesc")}
          badge={
            availableSkills.length > 0
              ? `(${availableSkills.filter((s) => !config.capabilities.skills.deny.includes(s.name)).length}/${availableSkills.length})`
              : undefined
          }
          open={skillsOpen}
          onToggle={() => setSkillsOpen((v) => !v)}
        >
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
        </CollapsibleSection>

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
