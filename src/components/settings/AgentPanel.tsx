import { useState, useEffect, useRef } from "react"
import { invoke, convertFileSrc } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { logger } from "@/lib/logger"
import { Switch } from "@/components/ui/switch"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { ModelSelector } from "@/components/ui/model-selector"
import { AvatarCropDialog } from "@/components/settings/AvatarCropDialog"
import MemoryPanel from "@/components/settings/MemoryPanel"
import SubagentPanelComponent from "@/components/settings/SubagentPanel"
import {
  ArrowDown,
  ArrowLeft,
  ArrowUp,
  Bot,
  Camera,
  Check,
  ChevronRight,
  Plus,
  Trash2,
  X,
} from "lucide-react"
import type {
  AgentSummary,
  AgentConfig,
  PersonalityConfig,
  AvailableModel,
  ActiveModelRef,
  SkillSummary,
} from "./types"
import { DEFAULT_PERSONALITY } from "./types"

const TONE_PRESETS = [
  { value: "formal", labelKey: "settings.agentToneFormal" },
  { value: "casual", labelKey: "settings.agentToneCasual" },
  { value: "playful", labelKey: "settings.agentTonePlayful" },
  { value: "professional", labelKey: "settings.agentToneProfessional" },
  { value: "warm", labelKey: "settings.agentToneWarm" },
  { value: "direct", labelKey: "settings.agentToneDirect" },
]

export default function AgentPanel({ initialAgentId }: { initialAgentId?: string }) {
  const { t } = useTranslation()
  const [agents, setAgents] = useState<AgentSummary[]>([])
  const [loading, setLoading] = useState(true)
  const [editingId, setEditingId] = useState<string | null>(initialAgentId ?? null)
  const [creating, setCreating] = useState(false)

  async function reload() {
    try {
      const list = await invoke<AgentSummary[]>("list_agents")
      setAgents(list)
    } catch (e) {
      logger.error("settings", "AgentPanel::loadAgents", "Failed to load agents", e)
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => { reload() }, [])

  if (editingId) {
    return (
      <AgentEditView
        agentId={editingId}
        onBack={() => { setEditingId(null); reload() }}
      />
    )
  }

  if (creating) {
    return (
      <AgentCreateView
        onBack={() => setCreating(false)}
        onCreated={(id) => { setCreating(false); setEditingId(id) }}
      />
    )
  }

  // ── Agent List View ───────────────────────────────────────────
  return (
    <div className="flex-1 min-h-0 overflow-y-auto p-6">
      <h2 className="text-lg font-semibold text-foreground mb-1">
        {t("settings.agents")}
      </h2>
      <p className="text-xs text-muted-foreground mb-4">
        {t("settings.agentsDesc")}
      </p>

      {/* New Agent button */}
      <button
        className="flex items-center gap-2 w-full px-3 py-2.5 rounded-lg text-sm text-primary hover:bg-primary/5 transition-colors mb-3"
        onClick={() => setCreating(true)}
      >
        <Plus className="h-4 w-4" />
        <span className="font-medium">{t("settings.agentNew")}</span>
      </button>

      <div className="border-t border-border mb-4" />

      {loading ? (
        <div className="flex items-center justify-center py-12">
          <div className="animate-spin h-5 w-5 border-2 border-foreground border-t-transparent rounded-full" />
        </div>
      ) : agents.length === 0 ? (
        <div className="text-center py-12">
          <Bot className="h-10 w-10 text-muted-foreground/30 mx-auto mb-3" />
          <p className="text-sm text-muted-foreground">{t("settings.agentNoAgents")}</p>
          <p className="text-xs text-muted-foreground/70 mt-1">{t("settings.agentNoAgentsHint")}</p>
        </div>
      ) : (
        <div className="space-y-1">
          {agents.map((agent) => (
            <button
              key={agent.id}
              className="flex items-center gap-3 w-full px-3 py-2.5 rounded-lg text-sm transition-colors text-foreground hover:bg-secondary/60 group"
              onClick={() => setEditingId(agent.id)}
            >
              {/* Avatar / fallback */}
              <div className="w-9 h-9 rounded-full bg-primary/15 flex items-center justify-center text-primary shrink-0 overflow-hidden">
                {agent.avatar ? (
                  <img src={agent.avatar.startsWith("/") ? convertFileSrc(agent.avatar) : agent.avatar} className="w-full h-full object-cover" alt="" />
                ) : (
                  <Bot className="h-5 w-5" />
                )}
              </div>

              {/* Name + emoji + description */}
              <div className="flex-1 text-left min-w-0">
                <div className="font-medium truncate flex items-center gap-2">
                  {agent.name}{agent.emoji ? ` ${agent.emoji}` : ""}
                  {agent.id === "default" && (
                    <span className="text-[10px] px-1.5 py-0.5 rounded bg-secondary text-muted-foreground font-medium">
                      {t("settings.agentDefault")}
                    </span>
                  )}
                </div>
                {agent.description && (
                  <div className="text-xs text-muted-foreground truncate">{agent.description}</div>
                )}
              </div>

              <ChevronRight
                className="h-4 w-4 text-muted-foreground/30 shrink-0 group-hover:text-muted-foreground/60 transition-colors"
              />
            </button>
          ))}
        </div>
      )}
    </div>
  )
}

// ── Agent Create View ───────────────────────────────────────────

function AgentCreateView({
  onBack,
  onCreated,
}: {
  onBack: () => void
  onCreated: (id: string) => void
}) {
  const { t } = useTranslation()
  const [id, setId] = useState("")
  const [name, setName] = useState("")
  const [error, setError] = useState("")

  const handleCreate = async () => {
    const trimmedId = id.trim().toLowerCase()
    if (!trimmedId) return
    if (!/^[a-z0-9][a-z0-9-]*$/.test(trimmedId)) {
      setError(t("settings.agentNewIdHint"))
      return
    }

    try {
      const config: AgentConfig = {
        name: name.trim() || trimmedId,
        description: null,
        emoji: "🤖",
        avatar: null,
        model: { primary: null, fallbacks: [] },
        skills: { allow: [], deny: [] },
        tools: { allow: [], deny: [] },
        personality: { ...DEFAULT_PERSONALITY },
        behavior: { maxToolRounds: 10, requireApproval: ["*"], sandbox: false, skillEnvCheck: true },
        useCustomPrompt: false,
      }
      await invoke("save_agent_config_cmd", { id: trimmedId, config })
      onCreated(trimmedId)
    } catch (e) {
      setError(String(e))
    }
  }

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-y-auto p-6">
      <div className="max-w-4xl">
        <Button
          variant="ghost"
          size="sm"
          onClick={onBack}
          className="gap-1.5 text-muted-foreground hover:text-foreground mb-4"
        >
          <ArrowLeft className="h-4 w-4" />
          <span>{t("settings.agents")}</span>
        </Button>

        <h2 className="text-lg font-semibold text-foreground mb-5">
          {t("settings.agentNew")}
        </h2>

        <div className="space-y-4">
          <div>
            <div className="text-xs font-medium text-muted-foreground mb-2 px-1">{t("settings.agentNewId")}</div>
            <Input
              className="bg-secondary/40 rounded-lg font-mono"
              value={id}
              onChange={(e) => { setId(e.target.value); setError("") }}
              placeholder={t("settings.agentNewIdPlaceholder")}
              autoFocus
            />
            <p className="text-[11px] text-muted-foreground/60 mt-1 px-1">{t("settings.agentNewIdHint")}</p>
          </div>

          <div>
            <div className="text-xs font-medium text-muted-foreground mb-2 px-1">{t("settings.agentName")}</div>
            <Input
              className="bg-secondary/40 rounded-lg"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={t("settings.agentNamePlaceholder")}
              onKeyDown={(e) => { if (e.key === "Enter") handleCreate() }}
            />
          </div>

          {error && (
            <p className="text-xs text-destructive px-1">{error}</p>
          )}

          <Button onClick={handleCreate} disabled={!id.trim()}>
            {t("common.add")}
          </Button>
        </div>
      </div>
    </div>
  )
}

// ── Agent Edit View ─────────────────────────────────────────────

type AgentTab = "identity" | "personality" | "behavior" | "model" | "memory" | "subagent" | "custom"

function AgentEditView({
  agentId,
  onBack,
}: {
  agentId: string
  onBack: () => void
}) {
  const { t, i18n } = useTranslation()
  const [config, setConfig] = useState<AgentConfig | null>(null)
  const [agentMd, setAgentMd] = useState("")
  const [persona, setPersona] = useState("")
  const [toolsGuide, setToolsGuide] = useState("")
  const [activeTab, setActiveTab] = useState<AgentTab>("identity")
  const [saving, setSaving] = useState(false)
  const [saved, setSaved] = useState(false)
  const [availableSkills, setAvailableSkills] = useState<SkillSummary[]>([])
  const [builtinTools, setBuiltinTools] = useState<{ name: string; description: string }[]>([])
  const [availableModels, setAvailableModels] = useState<AvailableModel[]>([])
  const [addingAgentFallback, setAddingAgentFallback] = useState(false)
  const [traitInput, setTraitInput] = useState("")
  const [principleInput, setPrincipleInput] = useState("")
  const [needsFillTemplate, setNeedsFillTemplate] = useState(false)
  const composingRef = useRef(false)

  useEffect(() => {
    async function load() {
      try {
        const [cfg, md, per, tg, skills, tools, models] = await Promise.all([
          invoke<AgentConfig>("get_agent_config", { id: agentId }),
          invoke<string | null>("get_agent_markdown", { id: agentId, file: "agent.md" }),
          invoke<string | null>("get_agent_markdown", { id: agentId, file: "persona.md" }),
          invoke<string | null>("get_agent_markdown", { id: agentId, file: "tools.md" }),
          invoke<SkillSummary[]>("get_skills"),
          invoke<{ name: string; description: string }[]>("list_builtin_tools"),
          invoke<AvailableModel[]>("get_available_models"),
        ])
        setAvailableModels(models)
        setAvailableSkills(skills.filter(s => s.enabled))
        setBuiltinTools(tools)
        // Ensure personality exists (for agents created before this field was added)
        if (!cfg.personality) {
          cfg.personality = { ...DEFAULT_PERSONALITY }
        }
        // Ensure subagents config exists
        if (!cfg.subagents) {
          cfg.subagents = { enabled: true, allowedAgents: [], deniedAgents: [], maxConcurrent: 5, defaultTimeoutSecs: 300, model: null }
        }
        setConfig(cfg)
        setAgentMd(md ?? "")
        setPersona(per ?? "")
        setToolsGuide(tg ?? "")
        // Flag: file never created → fill with template; empty string means user cleared it intentionally
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
      await invoke("save_agent_config_cmd", { id: agentId, config })
      await Promise.all([
        invoke("save_agent_markdown", { id: agentId, file: "agent.md", content: agentMd }),
        invoke("save_agent_markdown", { id: agentId, file: "persona.md", content: persona }),
        invoke("save_agent_markdown", { id: agentId, file: "tools.md", content: toolsGuide }),
      ])
      setSaved(true)
      setTimeout(() => setSaved(false), 2000)
    } catch (e) {
      logger.error("settings", "AgentPanel::saveAgent", "Failed to save agent", e)
    } finally {
      setSaving(false)
    }
  }

  const handleDelete = async () => {
    if (agentId === "default") return
    if (!confirm(t("settings.agentDeleteConfirm"))) return
    try {
      await invoke("delete_agent", { id: agentId })
      onBack()
    } catch (e) {
      logger.error("settings", "AgentPanel::deleteAgent", "Failed to delete agent", e)
    }
  }

  const [agentCropSrc, setAgentCropSrc] = useState<string | null>(null)

  const handleAvatarPick = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog")
      const selected = await open({
        filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "gif", "webp", "svg"] }],
        multiple: false,
      })
      if (selected) {
        setAgentCropSrc(convertFileSrc(selected as string))
      }
    } catch (e) {
      logger.error("settings", "AgentPanel::pickAvatar", "Failed to pick avatar", e)
    }
  }

  const handleAgentCropConfirm = async (blob: Blob) => {
    setAgentCropSrc(null)
    try {
      const buf = await blob.arrayBuffer()
      const bytes = new Uint8Array(buf)
      let binary = ""
      for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i])
      const base64 = window.btoa(binary)
      const fileName = `agent_${agentId}_${Date.now()}.png`
      const savedPath = await invoke<string>("save_avatar", { imageData: base64, fileName })
      updateConfig({ avatar: savedPath })
    } catch (e) {
      logger.error("settings", "AgentPanel::saveAvatar", "Failed to save avatar", e)
    }
  }

  const updateConfig = (patch: Partial<AgentConfig>) => {
    setConfig((prev) => prev ? { ...prev, ...patch } : prev)
  }

  const updatePersonality = (patch: Partial<PersonalityConfig>) => {
    setConfig((prev) => prev ? {
      ...prev,
      personality: { ...prev.personality, ...patch },
    } : prev)
  }

  const textInputProps = (getter: string, setter: (v: string) => void) => ({
    value: getter,
    onChange: (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => {
      setter(e.target.value)
    },
    onCompositionStart: () => { composingRef.current = true },
    onCompositionEnd: (e: React.CompositionEvent<HTMLInputElement | HTMLTextAreaElement>) => {
      composingRef.current = false
      setter((e.target as HTMLInputElement).value)
    },
  })

  /** i18n key map for built-in tool names */
  const toolI18nKey: Record<string, string> = {
    exec: "Exec", process: "Process", read: "Read", write: "Write",
    edit: "Edit", ls: "Ls", grep: "Grep", find: "Find",
    apply_patch: "ApplyPatch", web_search: "WebSearch", web_fetch: "WebFetch",
    save_memory: "SaveMemory", recall_memory: "RecallMemory",
    update_memory: "UpdateMemory", delete_memory: "DeleteMemory",
    manage_cron: "ManageCron", browser: "Browser",
    send_notification: "SendNotification", subagent: "Subagent",
  }
  const toolDisplayName = (name: string) => {
    const key = toolI18nKey[name]
    return key ? t(`settings.tool${key}Name`) : name
  }
  const toolDisplayDesc = (name: string) => {
    const key = toolI18nKey[name]
    return key ? t(`settings.tool${key}Desc`) : ""
  }

  /** Character counter for markdown textareas */
  const MAX_MD_CHARS = 20000
  const CharCounter = ({ value }: { value: string }) => {
    const len = value.length
    const isNear = len > MAX_MD_CHARS * 0.8
    const isOver = len > MAX_MD_CHARS
    return (
      <div className={`text-[11px] text-right mt-1 px-1 ${isOver ? "text-red-500" : isNear ? "text-amber-500" : "text-muted-foreground/40"}`}>
        {len.toLocaleString()} / {MAX_MD_CHARS.toLocaleString()} {isOver ? t("settings.charLimitExceeded") : ""}
      </div>
    )
  }

  /** Fetch a template file from backend by name and current locale */
  const fetchTemplate = async (name: "agent" | "persona") => {
    // Map i18n language to locale code for templates
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
      return await invoke<string>("get_agent_template", { name, locale })
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
  }, [needsFillTemplate, config])

  const handleEnableCustomPrompt = async () => {
    // Pre-fill with templates from files if empty
    if (!agentMd.trim()) {
      const tpl = await fetchTemplate("agent")
      if (tpl) setAgentMd(tpl)
    }
    if (!persona.trim()) {
      const tpl = await fetchTemplate("persona")
      if (tpl) setPersona(tpl)
    }
    updateConfig({ useCustomPrompt: true })
  }

  const TABS: { id: AgentTab; labelKey: string }[] = [
    { id: "identity", labelKey: "settings.agentIdentity" },
    { id: "personality", labelKey: "settings.agentPersonalityTab" },
    { id: "behavior", labelKey: "settings.agentBehavior" },
    { id: "model", labelKey: "settings.agentModel" },
    { id: "memory", labelKey: "settings.memory" },
    { id: "subagent", labelKey: "settings.subagentTitle" },
    { id: "custom", labelKey: "settings.agentCustomPrompt" },
  ]

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
        <div className="max-w-4xl">
          {/* Back button */}
          <button
            onClick={onBack}
            className="flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground transition-colors mb-4"
          >
            <ArrowLeft className="h-4 w-4" />
            <span>{t("settings.agents")}</span>
          </button>

          {/* Header: Avatar + Name */}
          <div className="flex items-center gap-4 mb-5">
            {/* Avatar */}
            <div
              className="w-14 h-14 rounded-full bg-secondary border border-border/50 flex items-center justify-center overflow-hidden hover:border-primary/30 transition-colors cursor-pointer shrink-0"
              onClick={handleAvatarPick}
            >
              {config.avatar ? (
                <img src={config.avatar.startsWith("/") ? convertFileSrc(config.avatar) : config.avatar} className="w-full h-full object-cover" alt="" />
              ) : (
                <Camera className="h-5 w-5 text-muted-foreground/40" />
              )}
            </div>

            <div className="flex-1 min-w-0">
              <h2 className="text-lg font-semibold text-foreground truncate">{config.name}</h2>
              {config.description && <p className="text-xs text-muted-foreground truncate">{config.description}</p>}
            </div>
          </div>

          {/* Agent avatar crop dialog */}
          {agentCropSrc && (
            <AvatarCropDialog
              open={!!agentCropSrc}
              imageSrc={agentCropSrc}
              onConfirm={handleAgentCropConfirm}
              onCancel={() => setAgentCropSrc(null)}
            />
          )}

          {/* Tabs */}
          <div className="flex gap-1 mb-5 border-b border-border pb-px">
            {TABS.map((tab) => (
              <button
                key={tab.id}
                className={cn(
                  "px-3 py-1.5 text-sm rounded-t-md transition-colors -mb-px",
                  activeTab === tab.id
                    ? "text-primary border-b-2 border-primary font-medium"
                    : "text-muted-foreground hover:text-foreground"
                )}
                onClick={() => setActiveTab(tab.id)}
              >
                {t(tab.labelKey)}
              </button>
            ))}
          </div>

          {/* ── Identity Tab ── */}
          {activeTab === "identity" && (
            <div className="space-y-4">
              {/* Name */}
              <div>
                <div className="text-xs font-medium text-muted-foreground mb-2 px-1">{t("settings.agentName")}</div>
                <Input
                  className="bg-secondary/40 rounded-lg"
                  {...textInputProps(config.name, (v) => updateConfig({ name: v }))}
                  placeholder={t("settings.agentNamePlaceholder")}
                />
              </div>

              {/* Description */}
              <div>
                <div className="text-xs font-medium text-muted-foreground mb-2 px-1">{t("settings.agentDescription")}</div>
                <Textarea
                  className="bg-secondary/40 rounded-lg resize-y leading-relaxed min-h-[50px]"
                  rows={2}
                  {...textInputProps(config.description ?? "", (v) => updateConfig({ description: v || null }))}
                  placeholder={t("settings.agentDescriptionPlaceholder")}
                />
              </div>

              {/* Emoji */}
              <div>
                <div className="text-xs font-medium text-muted-foreground mb-2 px-1">{t("settings.agentEmoji")}</div>
                <Input
                  className="bg-secondary/40 rounded-lg"
                  {...textInputProps(config.emoji ?? "", (v) => updateConfig({ emoji: v || null }))}
                  placeholder={t("settings.agentEmojiPlaceholder")}
                />
              </div>

              <div className="border-t border-border/50" />

              {/* Role */}
              <div>
                <div className="text-xs font-medium text-muted-foreground mb-2 px-1">{t("settings.agentRole")}</div>
                <Textarea
                  className="bg-secondary/40 rounded-lg resize-y leading-relaxed min-h-[60px]"
                  rows={3}
                  {...textInputProps(config.personality.role ?? "", (v) => updatePersonality({ role: v || null }))}
                  placeholder={t("settings.agentRolePlaceholder")}
                />
              </div>

              <div className="border-t border-border/50" />

              {/* Identity supplement */}
              <div>
                <div className="text-xs font-medium text-muted-foreground mb-1 px-1">{t("settings.agentSupplement")}</div>
                <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">{t("settings.agentIdentitySupplementDesc")}</p>
                <Textarea
                  className="bg-secondary/40 rounded-lg resize-y leading-relaxed font-mono min-h-[120px]"
                  rows={8}
                  {...textInputProps(agentMd, setAgentMd)}
                  placeholder={t("settings.agentSupplementPlaceholder")}
                />
                <CharCounter value={agentMd} />
              </div>
            </div>
          )}

          {/* ── Personality Tab ── */}
          {activeTab === "personality" && (
            <div className="space-y-5">
              {/* Vibe */}
              <div>
                <div className="text-xs font-medium text-muted-foreground mb-1 px-1">{t("settings.agentVibe")}</div>
                <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">{t("settings.agentVibeDesc")}</p>
                <Textarea
                  className="bg-secondary/40 rounded-lg resize-y leading-relaxed min-h-[60px]"
                  rows={3}
                  {...textInputProps(config.personality.vibe ?? "", (v) => updatePersonality({ vibe: v || null }))}
                  placeholder={t("settings.agentVibePlaceholder")}
                />
              </div>

              {/* Tone */}
              <div>
                <div className="text-xs font-medium text-muted-foreground mb-2 px-1">{t("settings.agentTone")}</div>
                <div className="flex flex-wrap gap-1.5 mb-2">
                  {TONE_PRESETS.map((preset) => (
                    <button
                      key={preset.value}
                      className={cn(
                        "px-2.5 py-1.5 text-xs rounded-md transition-colors",
                        config.personality.tone === preset.value
                          ? "bg-primary/10 text-primary font-medium"
                          : "bg-secondary/30 text-foreground hover:bg-secondary/60"
                      )}
                      onClick={() => updatePersonality({
                        tone: config.personality.tone === preset.value ? null : preset.value,
                      })}
                    >
                      {t(preset.labelKey)}
                    </button>
                  ))}
                </div>
                <Textarea
                  className="bg-secondary/40 rounded-lg resize-y leading-relaxed min-h-[60px]"
                  rows={3}
                  {...textInputProps(config.personality.tone ?? "", (v) => updatePersonality({ tone: v || null }))}
                  placeholder={t("settings.agentTonePlaceholder")}
                />
              </div>

              {/* Traits (tag input) */}
              <div>
                <div className="text-xs font-medium text-muted-foreground mb-1 px-1">{t("settings.agentTraits")}</div>
                <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">{t("settings.agentTraitsDesc")}</p>
                <div className="flex flex-wrap gap-1.5 mb-2">
                  {config.personality.traits.map((trait) => (
                    <span
                      key={trait}
                      className="inline-flex items-center gap-1 px-2 py-1 text-xs rounded-md bg-secondary text-foreground"
                    >
                      {trait}
                      <button
                        className="text-muted-foreground hover:text-destructive transition-colors"
                        onClick={() => updatePersonality({
                          traits: config.personality.traits.filter((t) => t !== trait),
                        })}
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
                <div className="text-xs font-medium text-muted-foreground mb-1 px-1">{t("settings.agentPrinciples")}</div>
                <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">{t("settings.agentPrinciplesDesc")}</p>
                <div className="space-y-1 mb-2">
                  {config.personality.principles.map((p, i) => (
                    <div
                      key={i}
                      className="flex items-center gap-2 px-2.5 py-1.5 text-xs rounded-md bg-secondary/30 text-foreground"
                    >
                      <span className="flex-1">{p}</span>
                      <button
                        className="text-muted-foreground hover:text-destructive transition-colors shrink-0"
                        onClick={() => updatePersonality({
                          principles: config.personality.principles.filter((_, idx) => idx !== i),
                        })}
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
                <div className="text-xs font-medium text-muted-foreground mb-1 px-1">{t("settings.agentBoundaries")}</div>
                <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">{t("settings.agentBoundariesDesc")}</p>
                <Textarea
                  className="bg-secondary/40 rounded-lg resize-y leading-relaxed min-h-[60px]"
                  rows={3}
                  {...textInputProps(config.personality.boundaries ?? "", (v) => updatePersonality({ boundaries: v || null }))}
                  placeholder={t("settings.agentBoundariesPlaceholder")}
                />
              </div>

              {/* Quirks */}
              <div>
                <div className="text-xs font-medium text-muted-foreground mb-1 px-1">{t("settings.agentQuirks")}</div>
                <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">{t("settings.agentQuirksDesc")}</p>
                <Textarea
                  className="bg-secondary/40 rounded-lg resize-y leading-relaxed min-h-[60px]"
                  rows={3}
                  {...textInputProps(config.personality.quirks ?? "", (v) => updatePersonality({ quirks: v || null }))}
                  placeholder={t("settings.agentQuirksPlaceholder")}
                />
              </div>

              {/* Communication Style */}
              <div>
                <div className="text-xs font-medium text-muted-foreground mb-1 px-1">{t("settings.agentCommStyle")}</div>
                <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">{t("settings.agentCommStyleDesc")}</p>
                <Textarea
                  className="bg-secondary/40 rounded-lg resize-y leading-relaxed min-h-[60px]"
                  rows={3}
                  {...textInputProps(config.personality.communicationStyle ?? "", (v) => updatePersonality({ communicationStyle: v || null }))}
                  placeholder={t("settings.agentCommStylePlaceholder")}
                />
              </div>

              <div className="border-t border-border/50" />

              {/* Personality supplement */}
              <div>
                <div className="text-xs font-medium text-muted-foreground mb-1 px-1">{t("settings.agentSupplement")}</div>
                <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">{t("settings.agentPersonaSupplementDesc")}</p>
                <Textarea
                  className="bg-secondary/40 rounded-lg resize-y leading-relaxed font-mono min-h-[120px]"
                  rows={8}
                  {...textInputProps(persona, setPersona)}
                  placeholder={t("settings.agentSupplementPlaceholder")}
                />
                <CharCounter value={persona} />
              </div>
            </div>
          )}

          {/* ── Behavior Tab ── */}
          {activeTab === "behavior" && (
            <div className="space-y-5">
              {/* Max Tool Rounds */}
              <div>
                <div className="text-xs font-medium text-muted-foreground mb-2 px-1">{t("settings.agentMaxToolRounds")}</div>
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
                      if (v > 0) updateConfig({ behavior: { ...config.behavior, maxToolRounds: v } })
                    }}
                  />
                  <label className="flex items-center gap-1.5 text-xs text-muted-foreground whitespace-nowrap cursor-pointer select-none">
                    <input
                      type="checkbox"
                      className="rounded"
                      checked={config.behavior.maxToolRounds === 0}
                      onChange={(e) => {
                        updateConfig({ behavior: { ...config.behavior, maxToolRounds: e.target.checked ? 0 : 10 } })
                      }}
                    />
                    {t("settings.agentUnlimited")}
                  </label>
                </div>
              </div>

              {/* Require Approval */}
              <div>
                <div className="text-xs font-medium text-muted-foreground mb-1 px-1">{t("settings.agentRequireApproval")}</div>
                <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">{t("settings.agentRequireApprovalDesc")}</p>
                {/* Mode selector */}
                <div className="flex gap-1.5 mb-3">
                  {([
                    { mode: "all", label: t("settings.agentApprovalAll") },
                    { mode: "none", label: t("settings.agentApprovalNone") },
                    { mode: "custom", label: t("settings.agentApprovalCustom") },
                  ] as const).map(({ mode, label }) => {
                    const currentMode = config.behavior.requireApproval.includes("*") ? "all"
                      : config.behavior.requireApproval.length === 0 ? "none" : "custom"
                    const isActive = currentMode === mode
                    return (
                      <button
                        key={mode}
                        className={cn(
                          "px-3 py-1.5 text-xs rounded-md border transition-colors",
                          isActive
                            ? "bg-primary/10 border-primary/40 text-primary"
                            : "bg-secondary/40 border-border/50 text-muted-foreground hover:border-border"
                        )}
                        onClick={() => {
                          if (mode === "all") {
                            updateConfig({ behavior: { ...config.behavior, requireApproval: ["*"] } })
                          } else if (mode === "none") {
                            updateConfig({ behavior: { ...config.behavior, requireApproval: [] } })
                          } else {
                            updateConfig({ behavior: { ...config.behavior, requireApproval: ["exec"] } })
                          }
                        }}
                      >
                        {label}
                      </button>
                    )
                  })}
                </div>
                {/* Custom tool selection */}
                {!config.behavior.requireApproval.includes("*") && config.behavior.requireApproval.length > 0 && (
                  <div className="rounded-lg border border-border/50 overflow-hidden">
                    {builtinTools.map((tool, idx) => {
                      const isRequired = config.behavior.requireApproval.includes(tool.name)
                      return (
                        <div
                          key={tool.name}
                          className={cn(
                            "flex items-center justify-between px-3 py-2 gap-3",
                            idx > 0 && "border-t border-border/30"
                          )}
                        >
                          <div className="min-w-0 flex-1">
                            <div className="text-xs font-medium text-foreground">{toolDisplayName(tool.name)}</div>
                            <div className="text-[11px] text-muted-foreground/60 line-clamp-1">{toolDisplayDesc(tool.name)}</div>
                          </div>
                          <Switch
                            checked={isRequired}
                            onCheckedChange={(checked) => {
                              const newList = checked
                                ? [...config.behavior.requireApproval, tool.name]
                                : config.behavior.requireApproval.filter(t => t !== tool.name)
                              updateConfig({ behavior: { ...config.behavior, requireApproval: newList.length > 0 ? newList : ["exec"] } })
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
                  <div className="text-xs text-muted-foreground">{t("settings.agentSandboxDesc")}</div>
                </div>
                <Switch
                  checked={config.behavior.sandbox}
                  onCheckedChange={(v) => updateConfig({ behavior: { ...config.behavior, sandbox: v } })}
                />
              </div>

              <div className="border-t border-border/50" />

              {/* Skills */}
              <div>
                <div className="text-xs font-medium text-muted-foreground mb-1 px-1">{t("settings.agentSkills")}</div>
                <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">{t("settings.agentSkillsDesc")}</p>
                {availableSkills.length > 0 && (
                  <div className="rounded-lg border border-border/50 overflow-hidden mb-3">
                    {availableSkills.map((skill, idx) => {
                      const isDenied = config.skills.deny.includes(skill.name)
                      return (
                        <div
                          key={skill.name}
                          className={cn(
                            "flex items-center justify-between px-3 py-2 gap-3",
                            idx > 0 && "border-t border-border/30"
                          )}
                        >
                          <div className="min-w-0 flex-1">
                            <div className="text-xs font-medium text-foreground truncate">{skill.name}</div>
                            <div className="text-[11px] text-muted-foreground/60 truncate">{skill.description}</div>
                          </div>
                          <Switch
                            checked={!isDenied}
                            onCheckedChange={(checked) => {
                              const newDeny = checked
                                ? config.skills.deny.filter(n => n !== skill.name)
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
                    <div className="text-sm text-foreground">{t("settings.agentSkillEnvCheck")}</div>
                    <div className="text-xs text-muted-foreground">{t("settings.agentSkillEnvCheckDesc")}</div>
                  </div>
                  <Switch
                    checked={config.behavior.skillEnvCheck ?? true}
                    onCheckedChange={(v) => updateConfig({ behavior: { ...config.behavior, skillEnvCheck: v } })}
                  />
                </div>
              </div>

              <div className="border-t border-border/50" />

              {/* Tool guidance */}
              <div>
                <div className="text-xs font-medium text-muted-foreground mb-1 px-1">{t("settings.agentToolsGuide")}</div>
                <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">{t("settings.agentToolsGuideDesc")}</p>
                <Textarea
                  className="bg-secondary/40 rounded-lg resize-y leading-relaxed font-mono min-h-[80px]"
                  rows={5}
                  {...textInputProps(toolsGuide, setToolsGuide)}
                  placeholder={t("settings.agentToolsGuidePlaceholder")}
                />
                <CharCounter value={toolsGuide} />
              </div>
            </div>
          )}

          {/* ── Memory Tab ── */}
          {activeTab === "memory" && (
            <MemoryPanel agentId={agentId} compact />
          )}

          {/* ── Sub-Agent Tab ── */}
          {activeTab === "subagent" && (
            <SubagentPanelComponent
              config={config.subagents}
              currentAgentId={agentId}
              onChange={(subagents) => updateConfig({ subagents })}
            />
          )}

          {/* ── Custom Prompt Tab ── */}
          {activeTab === "custom" && (
            <div className="space-y-5">
              {/* Toggle */}
              <div className="flex items-center justify-between px-1">
                <div>
                  <div className="text-sm text-foreground">{t("settings.agentCustomPrompt")}</div>
                  <div className="text-xs text-muted-foreground">{t("settings.agentCustomPromptDesc")}</div>
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
                    <p className="text-xs text-amber-600 dark:text-amber-400">{t("settings.agentCustomPromptWarning")}</p>
                  </div>

                  {/* Custom Identity */}
                  <div>
                    <div className="text-xs font-medium text-muted-foreground mb-1 px-1">{t("settings.agentMd")}</div>
                    <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">{t("settings.agentCustomIdentityDesc")}</p>
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
                    <div className="text-xs font-medium text-muted-foreground mb-1 px-1">{t("settings.agentPersona")}</div>
                    <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">{t("settings.agentCustomPersonaDesc")}</p>
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
          )}

          {/* ── Model Tab ── */}
          {activeTab === "model" && (() => {
            const isCustom = !!(config.model.primary)
            const modelDisplayName = (ref: string) => {
              const parts = ref.split("::")
              if (parts.length < 2) return ref
              const [pid, ...rest] = parts
              const mid = rest.join("::")
              const m = availableModels.find(m => m.providerId === pid && m.modelId === mid)
              return m ? `${m.providerName} / ${m.modelName}` : ref
            }
            const fallbacks = config.model.fallbacks || []
            const availableForFallback = availableModels.filter(
              m => {
                const ref = `${m.providerId}::${m.modelId}`
                return ref !== config.model.primary && !fallbacks.includes(ref)
              }
            )

            return (
              <div className="space-y-5">
                {/* Inherit / Custom toggle */}
                <div className="flex items-center justify-between px-1">
                  <div>
                    <div className="text-sm text-foreground">{t("settings.agentModelCustom")}</div>
                    <div className="text-xs text-muted-foreground">{t("settings.agentModelCustomDesc")}</div>
                  </div>
                  <Switch
                    checked={isCustom}
                    onCheckedChange={async (v) => {
                      if (v) {
                        // Inherit from global settings
                        try {
                          const [globalActive, globalFallbacks] = await Promise.all([
                            invoke<ActiveModelRef | null>("get_active_model"),
                            invoke<ActiveModelRef[]>("get_fallback_models"),
                          ])
                          const primary = globalActive
                            ? `${globalActive.providerId}::${globalActive.modelId}`
                            : (availableModels[0] ? `${availableModels[0].providerId}::${availableModels[0].modelId}` : null)
                          const fallbacks = globalFallbacks.map(f => `${f.providerId}::${f.modelId}`)
                          updateConfig({ model: { ...config.model, primary, fallbacks } })
                        } catch {
                          // Fallback: use first available model
                          const first = availableModels[0]
                          if (first) {
                            updateConfig({ model: { ...config.model, primary: `${first.providerId}::${first.modelId}` } })
                          }
                        }
                      } else {
                        updateConfig({ model: { primary: null, fallbacks: [] } })
                      }
                    }}
                  />
                </div>

                {!isCustom && (
                  <div className="rounded-lg border border-border/50 bg-secondary/20 px-3 py-2">
                    <p className="text-xs text-muted-foreground">{t("settings.agentModelInheritHint")}</p>
                  </div>
                )}

                {isCustom && (
                  <>
                    {/* Primary model selector */}
                    <div>
                      <div className="text-xs font-medium text-muted-foreground mb-1 px-1">{t("settings.agentModelPrimary")}</div>
                      <ModelSelector
                        value={config.model.primary || ""}
                        onChange={(providerId, modelId) => updateConfig({ model: { ...config.model, primary: `${providerId}::${modelId}` } })}
                        availableModels={availableModels}
                        placeholder={t("settings.selectDefaultModel")}
                      />
                    </div>

                    <div className="border-t border-border/50" />

                    {/* Fallback models */}
                    <div>
                      <div className="text-xs font-medium text-muted-foreground mb-1 px-1">{t("settings.fallbackModels")}</div>
                      <p className="text-[11px] text-muted-foreground/60 mb-3 px-1">{t("settings.fallbackModelsDesc")}</p>

                      {fallbacks.length === 0 ? (
                        <div className="text-center py-4 text-xs text-muted-foreground/50">{t("settings.noFallbackModels")}</div>
                      ) : (
                        <div className="space-y-1 mb-3">
                          {fallbacks.map((ref, i) => (
                            <div key={ref} className="flex items-center gap-2 px-3 py-2 rounded-lg bg-secondary/40">
                              <span className="text-[10px] px-1.5 py-0.5 rounded bg-primary/10 text-primary font-medium shrink-0">
                                #{i + 1}
                              </span>
                              <span className="text-sm text-foreground flex-1 truncate">{modelDisplayName(ref)}</span>
                              <div className="flex items-center gap-0.5 shrink-0">
                                <button
                                  className="p-0.5 text-muted-foreground hover:text-foreground transition-colors disabled:opacity-30"
                                  onClick={() => {
                                    if (i === 0) return
                                    const newList = [...fallbacks]
                                    ;[newList[i], newList[i - 1]] = [newList[i - 1], newList[i]]
                                    updateConfig({ model: { ...config.model, fallbacks: newList } })
                                  }}
                                  disabled={i === 0}
                                ><ArrowUp className="h-3 w-3" /></button>
                                <button
                                  className="p-0.5 text-muted-foreground hover:text-foreground transition-colors disabled:opacity-30"
                                  onClick={() => {
                                    if (i === fallbacks.length - 1) return
                                    const newList = [...fallbacks]
                                    ;[newList[i], newList[i + 1]] = [newList[i + 1], newList[i]]
                                    updateConfig({ model: { ...config.model, fallbacks: newList } })
                                  }}
                                  disabled={i === fallbacks.length - 1}
                                ><ArrowDown className="h-3 w-3" /></button>
                                <button
                                  className="p-0.5 text-muted-foreground hover:text-destructive transition-colors ml-1"
                                  onClick={() => {
                                    updateConfig({ model: { ...config.model, fallbacks: fallbacks.filter((_, j) => j !== i) } })
                                  }}
                                ><X className="h-3 w-3" /></button>
                              </div>
                            </div>
                          ))}
                        </div>
                      )}

                      {/* Add fallback button / selector */}
                      {!addingAgentFallback ? (
                        <Button
                          variant="ghost"
                          size="sm"
                          className="gap-1.5 text-primary hover:text-primary/80 px-1"
                          onClick={() => setAddingAgentFallback(true)}
                        >
                          <Plus className="h-3.5 w-3.5" />
                          <span>{t("settings.addFallbackModel")}</span>
                        </Button>
                      ) : (
                        <ModelSelector
                          defaultOpen={true}
                          onOpenChange={(open) => {
                            if (!open) setAddingAgentFallback(false)
                          }}
                          value=""
                          onChange={(providerId, modelId) => {
                            const ref = `${providerId}::${modelId}`
                            updateConfig({ model: { ...config.model, fallbacks: [...fallbacks, ref] } })
                            setAddingAgentFallback(false)
                          }}
                          availableModels={availableForFallback}
                          placeholder={t("settings.selectFallbackModel")}
                        />
                      )}
                    </div>
                  </>
                )}
              </div>
            )
          })()}
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
              onClick={handleDelete}
            >
              <Trash2 className="h-3.5 w-3.5" />
              <span>{t("common.delete")}</span>
            </Button>
          )}
        </div>
        <Button
          className={cn(
            saved && "bg-green-500/10 text-green-600 hover:bg-green-500/20"
          )}
          onClick={handleSave}
          disabled={saving}
        >
          {saving ? t("common.saving") : saved ? (
            <span className="flex items-center gap-1.5">
              <Check className="h-3.5 w-3.5" />
              {t("settings.agentSaved")}
            </span>
          ) : t("common.save")}
        </Button>
      </div>
    </div>
  )
}
