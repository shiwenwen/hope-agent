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
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { AvatarCropDialog } from "@/components/settings/AvatarCropDialog"
import { useAvatarUpload } from "@/hooks/useAvatarUpload"
import { ArrowLeft, Camera, Check, Loader2, Trash2 } from "lucide-react"
import type {
  AgentConfig,
  PersonalityConfig,
  AvailableModel,
  SkillSummary,
  AgentTab,
} from "./types"
import { DEFAULT_PERSONALITY, TABS } from "./types"
import IdentityTab from "./tabs/IdentityTab"
import PersonalityTab from "./tabs/PersonalityTab"
import CapabilitiesTab from "./tabs/CapabilitiesTab"
import ModelTab from "./tabs/ModelTab"
import MemoryTab from "./tabs/MemoryTab"
import SubagentTab from "./tabs/SubagentTab"
import CustomTab from "./tabs/CustomTab"

interface AgentEditViewProps {
  agentId: string
  onBack: () => void
}

export default function AgentEditView({ agentId, onBack }: AgentEditViewProps) {
  const { t, i18n } = useTranslation()
  const [config, setConfig] = useState<AgentConfig | null>(null)
  const [agentMd, setAgentMd] = useState("")
  const [persona, setPersona] = useState("")
  const [toolsGuide, setToolsGuide] = useState("")
  const [agentsMd, setAgentsMd] = useState("")
  const [identityMd, setIdentityMd] = useState("")
  const [soulMd, setSoulMd] = useState("")
  const [activeTab, setActiveTab] = useState<AgentTab>("identity")
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  const [availableSkills, setAvailableSkills] = useState<SkillSummary[]>([])
  const [builtinTools, setBuiltinTools] = useState<
    { name: string; description: string; internal?: boolean }[]
  >([])
  const [availableModels, setAvailableModels] = useState<AvailableModel[]>([])
  const [needsFillTemplate, setNeedsFillTemplate] = useState(false)
  const [confirmDeleteOpen, setConfirmDeleteOpen] = useState(false)
  const composingRef = useRef(false)

  useEffect(() => {
    async function load() {
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
            enabled: true,
            allowedAgents: [],
            deniedAgents: [],
            maxConcurrent: 5,
            defaultTimeoutSecs: 300,
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
        logger.error("settings", "AgentPanel::loadAgent", "Failed to load agent", e)
      }
    }
    load()
  }, [agentId])

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
      setTimeout(() => setSaveStatus("idle"), 2000)
    } finally {
      setSaving(false)
    }
  }

  const handleDelete = async () => {
    if (agentId === "default") return
    if (!config) return
    try {
      await getTransport().call("delete_agent", { id: agentId })
      window.dispatchEvent(new Event("agents-changed"))
      toast.success(t("common.deleted"), {
        description: config.name,
      })
      onBack()
    } catch (e) {
      logger.error("settings", "AgentPanel::deleteAgent", "Failed to delete agent", e)
      toast.error(t("common.deleteFailed"), {
        description: config.name,
      })
    }
    setConfirmDeleteOpen(false)
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
        <div>
          {agentId !== "default" && (
            <Button
              variant="ghost"
              size="sm"
              className="gap-1.5 text-muted-foreground hover:text-destructive"
              onClick={() => setConfirmDeleteOpen(true)}
            >
              <Trash2 className="h-3.5 w-3.5" />
              <span>{t("common.delete")}</span>
            </Button>
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
            <AlertDialogTitle>{t("settings.agentDeleteConfirm")}</AlertDialogTitle>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={() => void handleDelete()}
            >
              {t("common.delete")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
