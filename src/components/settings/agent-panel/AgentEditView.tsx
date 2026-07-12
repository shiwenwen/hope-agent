import { useState, useEffect, useRef } from "react"
import { toast } from "sonner"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { logger } from "@/lib/logger"
import { Button } from "@/components/ui/button"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Switch } from "@/components/ui/switch"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { AvatarCropDialog } from "@/components/settings/AvatarCropDialog"
import { useAvatarUpload } from "@/hooks/useAvatarUpload"
import { AlertTriangle, ArrowLeft, Camera, Check, Loader2, RefreshCw, Trash2 } from "lucide-react"
import type {
  AgentConfig,
  PersonalityConfig,
  AvailableModel,
  SkillSummary,
  AgentTab,
  AgentSummary,
} from "./types"
import { DEFAULT_PERSONALITY, TABS } from "./types"
import { isMainAgent } from "@/types/tools"
import IdentityTab from "./tabs/IdentityTab"
import PersonalityTab from "./tabs/PersonalityTab"
import CapabilitiesTab from "./tabs/CapabilitiesTab"
import ModelTab from "./tabs/ModelTab"
import MemoryTab from "./tabs/MemoryTab"
import SubagentTab from "./tabs/SubagentTab"
import ApprovalTab from "./tabs/ApprovalTab"
import CustomTab from "./tabs/CustomTab"
import {
  agentLoadOperationErrorToast,
  agentOperationErrorToast,
} from "./agentLoadOperationFeedback"

interface AgentEditViewProps {
  agentId: string
  initialTab?: AgentTab
  onBack: () => void
}

interface AgentDeletePreview {
  agentId: string
  agentName: string
  enabled: boolean
  isMain: boolean
  references: {
    globalConfig: number
    projects: number
    cronJobs: number
    pendingWakeups: number
    otherAgentConfigs: number
    historicalSessions: number
    historicalSubagentRuns: number
    historicalTeams: number
    agentMemories: number
  }
  activeWork: {
    agentRuns: number
    foregroundSessions: number
    subagentRuns: number
    teams: number
    cronRuns: number
    backgroundJobs: number
    pendingWakeups: number
  }
  hasHomeDir: boolean
  hasPlanDir: boolean
  blockers: string[]
}

export default function AgentEditView({ agentId, initialTab, onBack }: AgentEditViewProps) {
  const { t, i18n } = useTranslation()
  const [config, setConfig] = useState<AgentConfig | null>(null)
  const [agentMd, setAgentMd] = useState("")
  const [persona, setPersona] = useState("")
  const [toolsGuide, setToolsGuide] = useState("")
  const [agentsMd, setAgentsMd] = useState("")
  const [identityMd, setIdentityMd] = useState("")
  const [soulMd, setSoulMd] = useState("")
  const [activeTab, setActiveTab] = useState<AgentTab>(initialTab ?? "identity")
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  const [availableSkills, setAvailableSkills] = useState<SkillSummary[]>([])
  const [builtinTools, setBuiltinTools] = useState<
    {
      name: string
      description: string
      internal?: boolean
      tier?: "core" | "standard" | "configured" | "memory" | "mcp"
      core_subclass?: string | null
      default_for_main?: boolean | null
      default_for_others?: boolean | null
      config_hint?: string | null
    }[]
  >([])
  const [availableModels, setAvailableModels] = useState<AvailableModel[]>([])
  const [needsFillTemplate, setNeedsFillTemplate] = useState(false)
  const [confirmDeleteOpen, setConfirmDeleteOpen] = useState(false)
  const [deletePreview, setDeletePreview] = useState<AgentDeletePreview | null>(null)
  const [deleteLoading, setDeleteLoading] = useState(false)
  const [replacementAgentId, setReplacementAgentId] = useState("")
  const [replacementAgents, setReplacementAgents] = useState<AgentSummary[]>([])
  const [togglingEnabled, setTogglingEnabled] = useState(false)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [loadRetryKey, setLoadRetryKey] = useState(0)
  const composingRef = useRef(false)

  useEffect(() => {
    if (initialTab) setActiveTab(initialTab)
  }, [initialTab])

  useEffect(() => {
    let cancelled = false
    async function load() {
      setConfig(null)
      setLoadError(null)
      try {
        const [cfg, md, per, tg, skills, tools, models] = await Promise.all([
          getTransport().call<AgentConfig>("get_agent_config", { id: agentId }),
          getTransport().call<string | null>("get_agent_markdown", {
            id: agentId,
            file: "agent.md",
          }),
          getTransport().call<string | null>("get_agent_markdown", {
            id: agentId,
            file: "persona.md",
          }),
          getTransport().call<string | null>("get_agent_markdown", {
            id: agentId,
            file: "tools.md",
          }),
          getTransport().call<SkillSummary[]>("get_skills"),
          getTransport().call<{ name: string; description: string; internal?: boolean }[]>(
            "list_builtin_tools",
          ),
          getTransport().call<AvailableModel[]>("get_available_models"),
        ])
        // Fetch OpenClaw files only when mode is enabled.
        // SOUL.md is additionally loaded when the persona authoring surface is
        // SoulMd (non-openclaw users can still edit it via the Personality tab).
        let ocAgents: string | null = null,
          ocIdentity: string | null = null,
          ocSoul: string | null = null
        const personaSoulMode = cfg.personality?.mode === "soulMd"
        if (cfg.openclawMode) {
          ;[ocAgents, ocIdentity, ocSoul] = await Promise.all([
            getTransport().call<string | null>("get_agent_markdown", {
              id: agentId,
              file: "agents.md",
            }),
            getTransport().call<string | null>("get_agent_markdown", {
              id: agentId,
              file: "identity.md",
            }),
            getTransport().call<string | null>("get_agent_markdown", {
              id: agentId,
              file: "soul.md",
            }),
          ])
        } else if (personaSoulMode) {
          ocSoul = await getTransport().call<string | null>("get_agent_markdown", {
            id: agentId,
            file: "soul.md",
          })
        }
        if (cancelled) return
        setAvailableModels(models)
        setAvailableSkills(skills.filter((s) => s.enabled))
        setBuiltinTools(tools)
        // Ensure personality exists (for agents created before this field was added)
        if (!cfg.personality) {
          cfg.personality = { ...DEFAULT_PERSONALITY }
        }
        // Ensure subagents config exists
        if (!cfg.subagents) {
          cfg.subagents = {
            allowedAgents: [],
            deniedAgents: [],
            maxConcurrent: 8,
            defaultTimeoutSecs: 0,
            model: null,
          }
        }
        setConfig(cfg)
        setAgentMd(md ?? "")
        setPersona(per ?? "")
        setToolsGuide(tg ?? "")
        setAgentsMd(ocAgents ?? "")
        setIdentityMd(ocIdentity ?? "")
        setSoulMd(ocSoul ?? "")
        // Flag: file never created -> fill with template; empty string means user cleared it intentionally
        if (md === null || md === undefined) setNeedsFillTemplate(true)
      } catch (e) {
        if (cancelled) return
        logger.error("settings", "AgentPanel::loadAgent", "Failed to load agent", e)
        const failureToast = agentLoadOperationErrorToast(t, e)
        setLoadError(failureToast.description ?? failureToast.title)
        toast.error(
          failureToast.title,
          failureToast.description ? { description: failureToast.description } : undefined,
        )
      }
    }
    load()
    return () => {
      cancelled = true
    }
  }, [agentId, loadRetryKey, t])

  const handleSave = async () => {
    if (!config) return
    setSaving(true)
    try {
      await getTransport().call("save_agent_config_cmd", { id: agentId, config })
      const mdSaves = [
        getTransport().call("save_agent_markdown", {
          id: agentId,
          file: "agent.md",
          content: agentMd,
        }),
        getTransport().call("save_agent_markdown", {
          id: agentId,
          file: "persona.md",
          content: persona,
        }),
        getTransport().call("save_agent_markdown", {
          id: agentId,
          file: "tools.md",
          content: toolsGuide,
        }),
      ]
      // Only save OpenClaw files when mode is enabled. SOUL.md is saved
      // additionally in non-openclaw mode when the persona authoring surface
      // is set to SoulMd — the two surfaces share the same physical soul.md.
      if (config.openclawMode) {
        mdSaves.push(
          getTransport().call("save_agent_markdown", {
            id: agentId,
            file: "agents.md",
            content: agentsMd,
          }),
          getTransport().call("save_agent_markdown", {
            id: agentId,
            file: "identity.md",
            content: identityMd,
          }),
          getTransport().call("save_agent_markdown", {
            id: agentId,
            file: "soul.md",
            content: soulMd,
          }),
        )
      } else if (config.personality?.mode === "soulMd") {
        mdSaves.push(
          getTransport().call("save_agent_markdown", {
            id: agentId,
            file: "soul.md",
            content: soulMd,
          }),
        )
      }
      await Promise.all(mdSaves)
      window.dispatchEvent(new Event("agents-changed"))
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "AgentPanel::saveAgent", "Failed to save agent", e)
      setSaveStatus("failed")
      const failureToast = agentOperationErrorToast("save", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
      setTimeout(() => setSaveStatus("idle"), 2000)
    } finally {
      setSaving(false)
    }
  }

  const prepareDelete = async () => {
    if (isMainAgent(agentId)) return
    setConfirmDeleteOpen(true)
    setDeleteLoading(true)
    setDeletePreview(null)
    try {
      const [preview, agents] = await Promise.all([
        getTransport().call<AgentDeletePreview>("preview_agent_delete", { id: agentId }),
        getTransport().call<AgentSummary[]>("list_agents"),
      ])
      const choices = agents.filter((agent) => agent.id !== agentId && agent.enabled !== false)
      setDeletePreview(preview)
      setReplacementAgents(choices)
      setReplacementAgentId((current) =>
        choices.some((agent) => agent.id === current) ? current : (choices[0]?.id ?? ""),
      )
    } catch (e) {
      logger.error("settings", "AgentPanel::previewDelete", "Failed to preview deletion", e)
      const failureToast = agentOperationErrorToast("delete", t, e)
      toast.error(failureToast.title, { description: failureToast.description })
      setConfirmDeleteOpen(false)
    } finally {
      setDeleteLoading(false)
    }
  }

  const handleDelete = async () => {
    if (isMainAgent(agentId) || !config || !replacementAgentId) return
    setDeleteLoading(true)
    try {
      await getTransport().call("delete_agent", {
        id: agentId,
        replacementAgentId,
      })
      window.dispatchEvent(new Event("agents-changed"))
      toast.success(t("common.deleted"), {
        description: config.name,
      })
      onBack()
    } catch (e) {
      logger.error("settings", "AgentPanel::deleteAgent", "Failed to delete agent", e)
      const failureToast = agentOperationErrorToast("delete", t, e)
      toast.error(
        failureToast.title,
        failureToast.description
          ? { description: failureToast.description }
          : { description: config.name },
      )
    }
    setDeleteLoading(false)
    setConfirmDeleteOpen(false)
  }

  const handleEnabledChange = async (enabled: boolean) => {
    if (isMainAgent(agentId) || !config) return
    const previous = config.enabled !== false
    updateConfig({ enabled })
    setTogglingEnabled(true)
    try {
      await getTransport().call("set_agent_enabled", { id: agentId, enabled })
      window.dispatchEvent(new Event("agents-changed"))
      toast.success(enabled ? t("provider.enable") : t("provider.disable"), {
        description: config.name,
      })
    } catch (e) {
      updateConfig({ enabled: previous })
      const failureToast = agentOperationErrorToast("save", t, e)
      toast.error(failureToast.title, { description: failureToast.description })
    } finally {
      setTogglingEnabled(false)
    }
  }

  const {
    cropSrc: agentCropSrc,
    handleAvatarPick,
    handleCropCancel: handleAgentCropCancel,
    handleCropConfirm: handleAgentCropConfirm,
  } = useAvatarUpload({
    fileName: () => `agent_${agentId}_${Date.now()}.png`,
    logCategory: "AgentPanel",
    onSaved: (path) => updateConfig({ avatar: path }),
  })

  const updateConfig = (patch: Partial<AgentConfig>) => {
    setConfig((prev) => (prev ? { ...prev, ...patch } : prev))
  }

  const updatePersonality = (patch: Partial<PersonalityConfig>) => {
    setConfig((prev) =>
      prev
        ? {
            ...prev,
            personality: { ...prev.personality, ...patch },
          }
        : prev,
    )
  }

  const textInputProps = (getter: string, setter: (v: string) => void) => ({
    value: getter,
    onChange: (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => {
      setter(e.target.value)
    },
    onCompositionStart: () => {
      composingRef.current = true
    },
    onCompositionEnd: (e: React.CompositionEvent<HTMLInputElement | HTMLTextAreaElement>) => {
      composingRef.current = false
      setter((e.target as HTMLInputElement).value)
    },
  })

  /** Character counter for markdown textareas */
  const MAX_MD_CHARS = 20000
  const CharCounter = ({ value }: { value: string }) => {
    const len = value.length
    const isNear = len > MAX_MD_CHARS * 0.8
    const isOver = len > MAX_MD_CHARS
    return (
      <div
        className={`text-[11px] text-right mt-1 px-1 ${isOver ? "text-red-500" : isNear ? "text-amber-500" : "text-muted-foreground/40"}`}
      >
        {len.toLocaleString()} / {MAX_MD_CHARS.toLocaleString()}{" "}
        {isOver ? t("settings.charLimitExceeded") : ""}
      </div>
    )
  }

  /** Fetch a template file from backend by name and current locale */
  const fetchTemplate = async (name: string) => {
    const lang = i18n.language
    let locale = "en"
    if (lang.startsWith("zh-TW") || lang.startsWith("zh-HK")) locale = "zh-TW"
    else if (lang.startsWith("zh")) locale = "zh"
    else if (lang.startsWith("ja")) locale = "ja"
    else if (lang.startsWith("ko")) locale = "ko"
    else if (lang.startsWith("es")) locale = "es"
    else if (lang.startsWith("pt")) locale = "pt"
    else if (lang.startsWith("ru")) locale = "ru"
    else if (lang.startsWith("ar")) locale = "ar"
    else if (lang.startsWith("tr")) locale = "tr"
    else if (lang.startsWith("vi")) locale = "vi"
    else if (lang.startsWith("ms")) locale = "ms"
    try {
      return await getTransport().call<string>("get_agent_template", { name, locale })
    } catch {
      return ""
    }
  }

  // Fill empty agent.md with locale template after config loads
  useEffect(() => {
    if (needsFillTemplate && config) {
      fetchTemplate("agent").then((tpl) => {
        if (tpl) setAgentMd(tpl)
      })
      setNeedsFillTemplate(false)
    }
  }, [needsFillTemplate, config]) // eslint-disable-line react-hooks/exhaustive-deps

  const handleEnableOpenClawMode = async () => {
    // Pre-fill OpenClaw files with templates if empty
    const templateNames = [
      { state: agentsMd, setter: setAgentsMd, name: "openclaw_agents" },
      { state: identityMd, setter: setIdentityMd, name: "openclaw_identity" },
      { state: soulMd, setter: setSoulMd, name: "openclaw_soul" },
      { state: toolsGuide, setter: setToolsGuide, name: "openclaw_tools" },
    ]
    await Promise.all(
      templateNames.map(async ({ state, setter, name }) => {
        if (!state.trim()) {
          const tpl = await fetchTemplate(name)
          if (tpl) setter(tpl)
        }
      }),
    )
    updateConfig({ openclawMode: true })
  }

  if (loadError) {
    return (
      <div className="flex min-h-[360px] flex-1 items-center justify-center p-6">
        <div className="w-full max-w-md space-y-4 text-center">
          <div className="mx-auto flex h-10 w-10 items-center justify-center rounded-full border border-destructive/30 bg-destructive/10 text-destructive">
            <AlertTriangle className="h-5 w-5" />
          </div>
          <div className="space-y-1.5">
            <h2 className="text-base font-semibold text-foreground">
              {t("settings.agentLoadFailed")}
            </h2>
            <p className="text-sm text-muted-foreground">
              {t(
                "settings.agentLoadFailedHint",
                "The selected agent may have been removed or its configuration could not be read.",
              )}
            </p>
            <p className="break-words text-xs text-muted-foreground/80">{loadError}</p>
          </div>
          <div className="flex flex-wrap justify-center gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="gap-1.5"
              onClick={() => setLoadRetryKey((value) => value + 1)}
            >
              <RefreshCw className="h-3.5 w-3.5" />
              {t("common.retry")}
            </Button>
            <Button type="button" variant="ghost" size="sm" className="gap-1.5" onClick={onBack}>
              <ArrowLeft className="h-3.5 w-3.5" />
              {t("settings.agents")}
            </Button>
          </div>
        </div>
      </div>
    )
  }

  if (!config) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <div className="animate-spin h-5 w-5 border-2 border-foreground border-t-transparent rounded-full" />
      </div>
    )
  }

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-hidden">
      <div className="flex-1 overflow-y-auto p-6">
        <div className="w-full">
          {/* Back button */}
          <Button
            variant="ghost"
            size="sm"
            onClick={onBack}
            className="mb-4 -ml-3 gap-1.5 text-muted-foreground hover:text-foreground"
          >
            <ArrowLeft className="h-4 w-4" />
            <span>{t("settings.agents")}</span>
          </Button>

          {/* Header: Avatar + Name */}
          <div className="flex items-center gap-4 mb-5">
            {/* Avatar */}
            <div
              className="w-14 h-14 rounded-full bg-secondary border border-border/50 flex items-center justify-center overflow-hidden hover:border-primary/30 transition-colors cursor-pointer shrink-0"
              onClick={handleAvatarPick}
            >
              {config.avatar ? (
                <img
                  src={getTransport().resolveAssetUrl(config.avatar) ?? config.avatar}
                  className="w-full h-full object-cover"
                  alt=""
                />
              ) : (
                <Camera className="h-5 w-5 text-muted-foreground/40" />
              )}
            </div>

            <div className="flex-1 min-w-0">
              <h2 className="text-lg font-semibold text-foreground truncate">{config.name}</h2>
              {config.description && (
                <p className="text-xs text-muted-foreground truncate">{config.description}</p>
              )}
            </div>
          </div>

          {/* Agent avatar crop dialog */}
          {agentCropSrc && (
            <AvatarCropDialog
              open={!!agentCropSrc}
              imageSrc={agentCropSrc}
              onConfirm={handleAgentCropConfirm}
              onCancel={handleAgentCropCancel}
            />
          )}

          {/* Tabs */}
          <div className="flex gap-1 mb-5 border-b border-border pb-px">
            {TABS.map((tab) => (
              <Button
                key={tab.id}
                variant="ghost"
                size="sm"
                className={cn(
                  "h-auto rounded-t-md rounded-b-none px-3 py-1.5 text-sm font-normal -mb-px",
                  activeTab === tab.id
                    ? "text-primary border-b-2 border-primary font-medium hover:bg-transparent hover:text-primary"
                    : "text-muted-foreground hover:bg-transparent hover:text-foreground",
                )}
                onClick={() => setActiveTab(tab.id)}
              >
                {t(tab.labelKey)}
              </Button>
            ))}
          </div>

          {/* Tab content */}
          {activeTab === "identity" && (
            <IdentityTab
              config={config}
              agentMd={agentMd}
              openclawMode={config.openclawMode}
              updateConfig={updateConfig}
              updatePersonality={updatePersonality}
              setAgentMd={setAgentMd}
              textInputProps={textInputProps}
              CharCounter={CharCounter}
            />
          )}

          {activeTab === "personality" && (
            <PersonalityTab
              config={config}
              persona={persona}
              openclawMode={config.openclawMode}
              soulMd={soulMd}
              setSoulMd={setSoulMd}
              updatePersonality={updatePersonality}
              setPersona={setPersona}
              textInputProps={textInputProps}
              CharCounter={CharCounter}
            />
          )}

          {activeTab === "capabilities" && (
            <CapabilitiesTab
              config={config}
              agentId={agentId}
              builtinTools={builtinTools}
              availableSkills={availableSkills}
              toolsGuide={toolsGuide}
              openclawMode={config.openclawMode}
              updateConfig={updateConfig}
              setToolsGuide={setToolsGuide}
              textInputProps={textInputProps}
              CharCounter={CharCounter}
            />
          )}

          {activeTab === "model" && (
            <ModelTab
              config={config}
              availableModels={availableModels}
              updateConfig={updateConfig}
            />
          )}

          {activeTab === "memory" && (
            <MemoryTab
              agentId={agentId}
              openclawMode={config.openclawMode}
              config={config}
              updateConfig={updateConfig}
            />
          )}

          {activeTab === "subagent" && (
            <SubagentTab config={config} agentId={agentId} updateConfig={updateConfig} />
          )}

          {activeTab === "approval" && <ApprovalTab config={config} updateConfig={updateConfig} />}

          {activeTab === "custom" && (
            <CustomTab
              config={config}
              agentsMd={agentsMd}
              identityMd={identityMd}
              soulMd={soulMd}
              toolsGuide={toolsGuide}
              updateConfig={updateConfig}
              handleEnableOpenClawMode={handleEnableOpenClawMode}
              textInputProps={textInputProps}
              setAgentsMd={setAgentsMd}
              setIdentityMd={setIdentityMd}
              setSoulMd={setSoulMd}
              setToolsGuide={setToolsGuide}
              CharCounter={CharCounter}
            />
          )}
        </div>
      </div>

      {/* Bottom bar: delete + save */}
      <div className="shrink-0 flex items-center justify-between px-6 py-3 border-t border-border/30">
        <div className="flex items-center gap-3">
          {!isMainAgent(agentId) && (
            <>
              <div className="flex items-center gap-2 text-xs text-muted-foreground">
                <Switch
                  checked={config.enabled !== false}
                  disabled={togglingEnabled}
                  onCheckedChange={(enabled) => void handleEnabledChange(enabled)}
                  aria-label={
                    config.enabled !== false ? t("provider.disable") : t("provider.enable")
                  }
                />
                <span>
                  {config.enabled !== false
                    ? t("agentLifecycle.enabled")
                    : t("agentLifecycle.disabled")}
                </span>
              </div>
              <Button
                variant="ghost"
                size="sm"
                className="gap-1.5 text-muted-foreground hover:text-destructive"
                onClick={() => void prepareDelete()}
              >
                <Trash2 className="h-3.5 w-3.5" />
                <span>{t("common.delete")}</span>
              </Button>
            </>
          )}
        </div>
        <Button
          className={cn(
            saveStatus === "saved" && "bg-green-500/10 text-green-600 hover:bg-green-500/20",
            saveStatus === "failed" && "bg-destructive/10 text-destructive hover:bg-destructive/20",
          )}
          onClick={handleSave}
          disabled={saving}
        >
          {saving ? (
            <span className="flex items-center gap-1.5">
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              {t("common.saving")}
            </span>
          ) : saveStatus === "saved" ? (
            <span className="flex items-center gap-1.5">
              <Check className="h-3.5 w-3.5" />
              {t("common.saved")}
            </span>
          ) : saveStatus === "failed" ? (
            t("common.saveFailed")
          ) : (
            t("common.save")
          )}
        </Button>
      </div>

      <AlertDialog open={confirmDeleteOpen} onOpenChange={setConfirmDeleteOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("agentLifecycle.deleteTitle")}</AlertDialogTitle>
            <AlertDialogDescription>{t("agentLifecycle.deleteDescription")}</AlertDialogDescription>
          </AlertDialogHeader>
          {deleteLoading && !deletePreview ? (
            <div className="flex items-center justify-center py-8 text-muted-foreground">
              <Loader2 className="h-5 w-5 animate-spin" />
            </div>
          ) : deletePreview ? (
            <div className="space-y-4 text-sm">
              {deletePreview.blockers.includes("active_work") && (
                <div className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-destructive">
                  {t("agentLifecycle.activeWorkBlocked", {
                    count: Object.values(deletePreview.activeWork).reduce((a, b) => a + b, 0),
                  })}
                </div>
              )}
              <div className="grid grid-cols-2 gap-2 rounded-md bg-secondary/40 p-3 text-xs text-muted-foreground">
                <span>{t("agentLifecycle.routes")}</span>
                <span className="text-right text-foreground">
                  {deletePreview.references.globalConfig +
                    deletePreview.references.projects +
                    deletePreview.references.cronJobs +
                    deletePreview.references.pendingWakeups +
                    deletePreview.references.otherAgentConfigs}
                </span>
                <span>{t("agentLifecycle.history")}</span>
                <span className="text-right text-foreground">
                  {deletePreview.references.historicalSessions}
                </span>
                <span>{t("agentLifecycle.memories")}</span>
                <span className="text-right text-foreground">
                  {deletePreview.references.agentMemories}
                </span>
              </div>
              <div className="space-y-1.5">
                <div className="text-xs font-medium">{t("agentLifecycle.replacement")}</div>
                <Select value={replacementAgentId} onValueChange={setReplacementAgentId}>
                  <SelectTrigger>
                    <SelectValue placeholder={t("common.select")} />
                  </SelectTrigger>
                  <SelectContent>
                    {replacementAgents.map((agent) => (
                      <SelectItem key={agent.id} value={agent.id}>
                        {agent.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <p className="text-xs leading-relaxed text-muted-foreground">
                {t("agentLifecycle.retentionNotice")}
              </p>
            </div>
          ) : null}
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={() => void handleDelete()}
              disabled={
                deleteLoading ||
                !deletePreview ||
                deletePreview.blockers.length > 0 ||
                !replacementAgentId
              }
            >
              {deleteLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : t("common.delete")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
