import { useState, useEffect, useRef } from "react"
import { invoke, convertFileSrc } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import {
  DndContext,
  closestCenter,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core"
import {
  SortableContext,
  useSortable,
  verticalListSortingStrategy,
  arrayMove,
} from "@dnd-kit/sortable"
import { CSS } from "@dnd-kit/utilities"
import {
  ArrowDown,
  ArrowLeft,
  ArrowUp,
  Bot,
  Camera,
  Check,
  ChevronRight,
  Globe,
  GripVertical,
  Info,
  Layers,
  MessageSquare,
  Monitor,
  Moon,
  Palette,
  Plus,
  Puzzle,
  Server,
  Sun,
  Trash2,
  FolderOpen,
  File,
  Folder,
  ExternalLink,
  X,
  User,
} from "lucide-react"
import { SUPPORTED_LANGUAGES, isFollowingSystem, setFollowSystemLanguage } from "@/i18n/i18n"
import { useTheme, type ThemeMode } from "@/hooks/useTheme"
import { Switch } from "@/components/ui/switch"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { Select, SelectContent, SelectGroup, SelectItem, SelectLabel, SelectTrigger, SelectValue } from "@/components/ui/select"
import { ModelSelector } from "@/components/ui/model-selector"
import ProviderSettings from "@/components/settings/ProviderSettings"
import type { ProviderConfig } from "@/components/settings/ProviderSettings"
import ProviderSetup from "@/components/settings/ProviderSetup"
import ProviderEditPage from "@/components/settings/ProviderEditPage"
import { AvatarCropDialog } from "@/components/settings/AvatarCropDialog"

type SettingsSection = "providers" | "models" | "skills" | "agents" | "profile" | "chat" | "appearance" | "language" | "about"

interface SettingsSectionItem {
  id: SettingsSection
  icon: React.ReactNode
  labelKey: string
}

const SECTIONS: SettingsSectionItem[] = [
  {
    id: "profile",
    icon: <User className="h-4 w-4" />,
    labelKey: "settings.profile",
  },
  {
    id: "providers",
    icon: <Server className="h-4 w-4" />,
    labelKey: "settings.providers",
  },
  {
    id: "models",
    icon: <Layers className="h-4 w-4" />,
    labelKey: "settings.globalModel",
  },
  {
    id: "agents",
    icon: <Bot className="h-4 w-4" />,
    labelKey: "settings.agents",
  },
  {
    id: "skills",
    icon: <Puzzle className="h-4 w-4" />,
    labelKey: "settings.skills",
  },
  {
    id: "chat",
    icon: <MessageSquare className="h-4 w-4" />,
    labelKey: "settings.chat",
  },
  {
    id: "appearance",
    icon: <Palette className="h-4 w-4" />,
    labelKey: "settings.appearance",
  },
  {
    id: "language",
    icon: <Globe className="h-4 w-4" />,
    labelKey: "settings.language",
  },
  {
    id: "about",
    icon: <Info className="h-4 w-4" />,
    labelKey: "settings.about",
  },
]

// ── Chat Settings Panel ──────────────────────────────────────────

interface ChatConfig {
  autoSendPending: boolean
}

function ChatSettingsPanel() {
  const { t } = useTranslation()
  const [config, setConfig] = useState<ChatConfig>({ autoSendPending: true })
  const [loaded, setLoaded] = useState(false)

  useEffect(() => {
    invoke<{ autoSendPending?: boolean }>("get_user_config").then((cfg) => {
      setConfig({ autoSendPending: cfg.autoSendPending !== false })
      setLoaded(true)
    }).catch(console.error)
  }, [])

  async function toggle(key: keyof ChatConfig) {
    const updated = { ...config, [key]: !config[key] }
    setConfig(updated)
    try {
      const full = await invoke<Record<string, unknown>>("get_user_config")
      await invoke("save_user_config", { config: { ...full, ...updated } })
    } catch (e) {
      console.error("Failed to save chat config:", e)
    }
  }

  if (!loaded) return null

  return (
    <div className="space-y-4">
      <div
        className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors cursor-pointer"
        onClick={() => toggle("autoSendPending")}
      >
        <div className="space-y-0.5">
          <div className="text-sm font-medium">{t("settings.chatAutoSend")}</div>
          <div className="text-xs text-muted-foreground">{t("settings.chatAutoSendDesc")}</div>
        </div>
        <Switch checked={config.autoSendPending} onCheckedChange={() => toggle("autoSendPending")} />
      </div>
    </div>
  )
}

// ── Appearance Settings Panel ─────────────────────────────────────

const THEME_OPTIONS: { mode: ThemeMode; icon: React.ReactNode; labelKey: string; descKey: string }[] = [
  { mode: "auto", icon: <Monitor className="h-5 w-5" />, labelKey: "theme.auto", descKey: "theme.autoDesc" },
  { mode: "light", icon: <Sun className="h-5 w-5" />, labelKey: "theme.light", descKey: "theme.lightDesc" },
  { mode: "dark", icon: <Moon className="h-5 w-5" />, labelKey: "theme.dark", descKey: "theme.darkDesc" },
]

function AppearancePanel() {
  const { t } = useTranslation()
  const { theme, setTheme } = useTheme()

  return (
    <div className="flex-1 overflow-y-auto p-6 max-w-4xl">
      <h2 className="text-lg font-semibold text-foreground mb-1">
        {t("settings.appearance")}
      </h2>
      <p className="text-xs text-muted-foreground mb-5">
        {t("settings.appearanceDesc")}
      </p>

      <div className="space-y-1">
        {THEME_OPTIONS.map((opt) => (
          <button
            key={opt.mode}
            className={cn(
              "flex items-center gap-3 w-full px-3 py-3 rounded-lg text-sm transition-colors",
              theme === opt.mode
                ? "bg-primary/10 text-primary font-medium"
                : "text-foreground hover:bg-secondary/60"
            )}
            onClick={() => setTheme(opt.mode)}
          >
            <span
              className={cn(
                "shrink-0",
                theme === opt.mode ? "text-primary" : "text-muted-foreground"
              )}
            >
              {opt.icon}
            </span>
            <div className="flex-1 text-left">
              <div>{t(opt.labelKey)}</div>
              <div className="text-xs text-muted-foreground font-normal">
                {t(opt.descKey)}
              </div>
            </div>
            {theme === opt.mode && (
              <Check className="h-4 w-4 text-primary shrink-0" />
            )}
          </button>
        ))}
      </div>
    </div>
  )
}

// ── Language Settings Panel ───────────────────────────────────────

function LanguagePanel() {
  const { t, i18n } = useTranslation()
  const [followSystem, setFollowSystem] = useState(isFollowingSystem)

  const isCurrentLang = (code: string) => {
    if (followSystem) return false
    return (
      i18n.language === code ||
      (i18n.language.startsWith(code + "-") && code !== "zh")
    )
  }

  const handleFollowSystem = () => {
    setFollowSystemLanguage()
    setFollowSystem(true)
  }

  const handleSelectLanguage = (code: string) => {
    i18n.changeLanguage(code)
    setFollowSystem(false)
  }

  return (
    <div className="flex-1 overflow-y-auto p-6 max-w-4xl">
      <h2 className="text-lg font-semibold text-foreground mb-1">
        {t("settings.language")}
      </h2>
      <p className="text-xs text-muted-foreground mb-5">
        {t("settings.languageDesc")}
      </p>

      <div className="space-y-0.5">
        {/* Follow System option */}
        <button
          className={cn(
            "flex items-center gap-3 w-full px-3 py-2.5 rounded-lg text-sm transition-colors",
            followSystem
              ? "bg-primary/10 text-primary font-medium"
              : "text-foreground hover:bg-secondary/60"
          )}
          onClick={handleFollowSystem}
        >
          <span className={cn("shrink-0", followSystem ? "text-primary" : "text-muted-foreground")}>
            <Monitor className="h-4 w-4" />
          </span>
          <span className="flex-1 text-left">{t("language.system")}</span>
          {followSystem && (
            <Check className="h-4 w-4 text-primary shrink-0" />
          )}
        </button>

        {/* Divider */}
        <div className="border-t border-border/50 my-1.5" />

        {SUPPORTED_LANGUAGES.map((lang) => (
          <button
            key={lang.code}
            className={cn(
              "flex items-center gap-3 w-full px-3 py-2.5 rounded-lg text-sm transition-colors",
              isCurrentLang(lang.code)
                ? "bg-primary/10 text-primary font-medium"
                : "text-foreground hover:bg-secondary/60"
            )}
            onClick={() => handleSelectLanguage(lang.code)}
          >
            <span className="text-xs font-bold w-6 text-center opacity-60">
              {lang.shortLabel}
            </span>
            <span className="flex-1 text-left">{lang.label}</span>
            {isCurrentLang(lang.code) && (
              <Check className="h-4 w-4 text-primary shrink-0" />
            )}
          </button>
        ))}
      </div>
    </div>
  )
}

// ── Sortable Fallback Item ───────────────────────────────────────

function SortableFallbackItem({
  id,
  index,
  displayName,
  onRemove,
}: {
  id: string
  index: number
  displayName: string
  onRemove: () => void
}) {
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id })

  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.4 : 1,
    zIndex: isDragging ? 50 : undefined,
  }

  return (
    <div
      ref={setNodeRef}
      style={style}
      className="flex items-center gap-2 px-3 py-2 rounded-lg bg-secondary/30 border border-border/30 group"
    >
      {/* Drag handle */}
      <div
        className="cursor-grab active:cursor-grabbing text-muted-foreground/40 hover:text-muted-foreground/70 shrink-0 touch-none"
        {...attributes}
        {...listeners}
      >
        <GripVertical className="h-3.5 w-3.5" />
      </div>

      {/* Priority badge */}
      <span className="text-[10px] font-bold text-muted-foreground/50 w-5 text-center shrink-0">
        #{index + 1}
      </span>

      {/* Model name */}
      <span className="flex-1 text-sm text-foreground truncate">
        {displayName}
      </span>

      {/* Remove */}
      <button
        className="text-muted-foreground/40 hover:text-destructive transition-colors opacity-0 group-hover:opacity-100"
        onClick={onRemove}
      >
        <X className="h-3.5 w-3.5" />
      </button>
    </div>
  )
}

// ── Global Model Settings Panel ──────────────────────────────────

interface AvailableModel {
  providerId: string
  providerName: string
  apiType: string
  modelId: string
  modelName: string
  inputTypes: string[]
  contextWindow: number
  maxTokens: number
  reasoning: boolean
}

interface ActiveModelRef {
  providerId: string
  modelId: string
}

function GlobalModelPanel() {
  const { t } = useTranslation()
  const [availableModels, setAvailableModels] = useState<AvailableModel[]>([])
  const [activeModel, setActiveModel] = useState<ActiveModelRef | null>(null)
  const [fallbackModels, setFallbackModels] = useState<ActiveModelRef[]>([])
  const [loading, setLoading] = useState(true)
  const [addingFallback, setAddingFallback] = useState(false)

  useEffect(() => {
    async function load() {
      try {
        const [models, active, fallbacks] = await Promise.all([
          invoke<AvailableModel[]>("get_available_models"),
          invoke<ActiveModelRef | null>("get_active_model"),
          invoke<ActiveModelRef[]>("get_fallback_models"),
        ])
        setAvailableModels(models)
        setActiveModel(active)
        setFallbackModels(fallbacks)
      } catch (e) {
        console.error("Failed to load model settings:", e)
      } finally {
        setLoading(false)
      }
    }
    load()
  }, [])

  const modelDisplayName = (ref: ActiveModelRef) => {
    const m = availableModels.find(
      (m) => m.providerId === ref.providerId && m.modelId === ref.modelId
    )
    return m ? `${m.providerName} / ${m.modelName}` : `${ref.providerId}::${ref.modelId}`
  }

  const handleSetDefault = async (providerId: string, modelId: string) => {
    try {
      await invoke("set_active_model", { providerId, modelId })
      setActiveModel({ providerId, modelId })
    } catch (e) {
      console.error("Failed to set default model:", e)
    }
  }

  const handleSaveFallbacks = async (newFallbacks: ActiveModelRef[]) => {
    try {
      await invoke("set_fallback_models", { models: newFallbacks })
      setFallbackModels(newFallbacks)
    } catch (e) {
      console.error("Failed to save fallback models:", e)
    }
  }

  const handleAddFallback = (providerId: string, modelId: string) => {
    // Avoid duplicates
    if (fallbackModels.some((f) => f.providerId === providerId && f.modelId === modelId)) return
    const newList = [...fallbackModels, { providerId, modelId }]
    handleSaveFallbacks(newList)
    setAddingFallback(false)
  }

  const handleRemoveFallback = (index: number) => {
    const newList = fallbackModels.filter((_, i) => i !== index)
    handleSaveFallbacks(newList)
  }

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 5 } }),
  )

  const handleFallbackDragEnd = (event: DragEndEvent) => {
    const { active, over } = event
    if (!over || active.id === over.id) return
    const oldIndex = fallbackModels.findIndex(
      (f) => `${f.providerId}::${f.modelId}` === active.id
    )
    const newIndex = fallbackModels.findIndex(
      (f) => `${f.providerId}::${f.modelId}` === over.id
    )
    if (oldIndex === -1 || newIndex === -1) return
    const updated = arrayMove(fallbackModels, oldIndex, newIndex)
    handleSaveFallbacks(updated)
  }

  // Available for adding as fallback (not already in list, not the active model)
  const availableForFallback = availableModels.filter(
    (m) =>
      !fallbackModels.some(
        (f) => f.providerId === m.providerId && f.modelId === m.modelId
      ) &&
      !(activeModel?.providerId === m.providerId && activeModel?.modelId === m.modelId)
  )

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <div className="animate-spin h-5 w-5 border-2 border-foreground border-t-transparent rounded-full" />
      </div>
    )
  }

  return (
    <div className="flex-1 overflow-y-auto p-6 max-w-4xl">
      <h2 className="text-lg font-semibold text-foreground mb-1">
        {t("settings.globalModel")}
      </h2>
      <p className="text-xs text-muted-foreground mb-5">
        {t("settings.globalModelDesc")}
      </p>

      {/* Default Model */}
      <div className="mb-6">
        <div className="text-xs font-medium text-muted-foreground mb-1 px-1">
          {t("settings.defaultModel")}
        </div>
        <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">
          {t("settings.defaultModelDesc")}
        </p>

        <ModelSelector
          value={activeModel ? `${activeModel.providerId}::${activeModel.modelId}` : ""}
          onChange={(providerId, modelId) => handleSetDefault(providerId, modelId)}
          availableModels={availableModels}
          placeholder={t("settings.selectDefaultModel")}
        />
      </div>

      <div className="border-t border-border/50 mb-6" />

      {/* Fallback Models */}
      <div>
        <div className="text-xs font-medium text-muted-foreground mb-1 px-1">
          {t("settings.fallbackModels")}
        </div>
        <p className="text-[11px] text-muted-foreground/60 mb-3 px-1">
          {t("settings.fallbackModelsDesc")}
        </p>

        {fallbackModels.length === 0 ? (
          <div className="text-center py-6 bg-secondary/20 rounded-lg border border-border/30">
            <Layers className="h-8 w-8 text-muted-foreground/20 mx-auto mb-2" />
            <p className="text-xs text-muted-foreground/60">
              {t("settings.noFallbacks")}
            </p>
          </div>
        ) : (
          <DndContext
            sensors={sensors}
            collisionDetection={closestCenter}
            onDragEnd={handleFallbackDragEnd}
          >
            <SortableContext
              items={fallbackModels.map((f) => `${f.providerId}::${f.modelId}`)}
              strategy={verticalListSortingStrategy}
            >
              <div className="space-y-1.5 mb-3">
                {fallbackModels.map((fb, idx) => (
                  <SortableFallbackItem
                    key={`${fb.providerId}::${fb.modelId}`}
                    id={`${fb.providerId}::${fb.modelId}`}
                    index={idx}
                    displayName={modelDisplayName(fb)}
                    onRemove={() => handleRemoveFallback(idx)}
                  />
                ))}
              </div>
            </SortableContext>
          </DndContext>
        )}

        {/* Add fallback */}
        {addingFallback ? (
          <ModelSelector
            defaultOpen={true}
            onOpenChange={(open) => {
              if (!open) setAddingFallback(false)
            }}
            value=""
            onChange={(providerId, modelId) => handleAddFallback(providerId, modelId)}
            availableModels={availableForFallback}
            placeholder={t("settings.selectFallbackModel")}
          />

        ) : (
          <button
            className="flex items-center gap-1.5 text-xs text-primary hover:text-primary/80 transition-colors px-1 py-1.5"
            onClick={() => setAddingFallback(true)}
          >
            <Plus className="h-3.5 w-3.5" />
            <span>{t("settings.addFallback")}</span>
          </button>
        )}
      </div>
    </div>
  )
}

// ── Skills Panel ──────────────────────────────────────────────────

interface SkillSummary {
  name: string
  description: string
  source: string
  base_dir: string
  enabled: boolean
}

interface SkillFileInfo {
  name: string
  size: number
  is_dir: boolean
}

interface SkillDetail {
  name: string
  description: string
  source: string
  file_path: string
  base_dir: string
  content: string
  enabled: boolean
  files: SkillFileInfo[]
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

function SkillsPanel() {
  const { t } = useTranslation()
  const [skills, setSkills] = useState<SkillSummary[]>([])
  const [extraDirs, setExtraDirs] = useState<string[]>([])
  const [selectedSkill, setSelectedSkill] = useState<SkillDetail | null>(null)
  const [loading, setLoading] = useState(true)
  const [skillEnvCheck, setSkillEnvCheck] = useState(true)

  async function reload() {
    try {
      const [list, dirs, envCheck] = await Promise.all([
        invoke<SkillSummary[]>("get_skills"),
        invoke<string[]>("get_extra_skills_dirs"),
        invoke<boolean>("get_skill_env_check"),
      ])
      setSkills(list)
      setExtraDirs(dirs)
      setSkillEnvCheck(envCheck)
    } catch (e) {
      console.error("Failed to load skills:", e)
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => { reload() }, [])

  async function handleOpenDir(path: string) {
    try {
      await invoke("open_directory", { path })
    } catch (e) {
      console.error("Failed to open directory:", e)
    }
  }

  async function handleAddDir() {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog")
      const selected = await open({ directory: true, multiple: false })
      if (selected) {
        await invoke("add_extra_skills_dir", { dir: selected })
        await reload()
      }
    } catch (e) {
      console.error("Failed to add skills directory:", e)
    }
  }

  async function handleRemoveDir(dir: string) {
    try {
      await invoke("remove_extra_skills_dir", { dir })
      await reload()
    } catch (e) {
      console.error("Failed to remove skills directory:", e)
    }
  }

  async function handleToggleSkill(name: string, enabled: boolean) {
    try {
      await invoke("toggle_skill", { name, enabled })
      // Update local state immediately
      setSkills((prev) =>
        prev.map((s) => (s.name === name ? { ...s, enabled } : s))
      )
      if (selectedSkill?.name === name) {
        setSelectedSkill((prev) => prev ? { ...prev, enabled } : prev)
      }
    } catch (e) {
      console.error("Failed to toggle skill:", e)
    }
  }

  async function handleSelectSkill(name: string) {
    try {
      const detail = await invoke<SkillDetail>("get_skill_detail", { name })
      setSelectedSkill(detail)
    } catch (e) {
      console.error("Failed to load skill detail:", e)
    }
  }

  // ── Skill Detail View ──────────────────────────────────────────
  if (selectedSkill) {
    return (
      <div className="flex-1 flex flex-col min-h-0 overflow-y-auto p-6">
        <div className="max-w-4xl">
          <button
            onClick={() => setSelectedSkill(null)}
            className="flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground transition-colors mb-4"
          >
            <ArrowLeft className="h-4 w-4" />
            <span>{t("settings.skills")}</span>
          </button>

          {/* Header */}
          <div className="mb-4">
            <div className="flex items-center gap-3">
              <h2 className="text-lg font-semibold text-foreground">{selectedSkill.name}</h2>
              <Switch
                checked={selectedSkill.enabled}
                onCheckedChange={(v) => handleToggleSkill(selectedSkill.name, v)}
              />
            </div>
            <p className="text-xs text-muted-foreground mt-1">{selectedSkill.description}</p>
            <div className="flex items-center gap-2 mt-2">
              <span className="text-[10px] px-1.5 py-0.5 rounded bg-secondary text-muted-foreground font-medium">
                {selectedSkill.source}
              </span>
              <button
                className="flex items-center gap-1 text-[10px] text-muted-foreground hover:text-foreground transition-colors"
                onClick={() => handleOpenDir(selectedSkill.base_dir)}
                title={selectedSkill.base_dir}
              >
                <ExternalLink className="h-3 w-3" />
                <span className="truncate max-w-[300px]">{selectedSkill.base_dir}</span>
              </button>
            </div>
          </div>

          {/* Files in skill directory */}
          {selectedSkill.files.length > 0 && (
            <div className="mb-4">
              <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">
                {t("settings.skillFiles")}
              </h3>
              <div className="rounded-lg border border-border overflow-hidden">
                {selectedSkill.files.map((file) => (
                  <div
                    key={file.name}
                    className="flex items-center gap-2 px-3 py-1.5 text-xs border-b border-border/50 last:border-b-0 bg-secondary/20"
                  >
                    {file.is_dir
                      ? <Folder className="h-3.5 w-3.5 text-primary/60 shrink-0" />
                      : <File className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
                    }
                    <span className="flex-1 text-foreground/80 truncate">{file.name}{file.is_dir ? "/" : ""}</span>
                    {!file.is_dir && (
                      <span className="text-muted-foreground/60 shrink-0">{formatFileSize(file.size)}</span>
                    )}
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* SKILL.md content */}
          <div className="border-t border-border pt-4">
            <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">SKILL.md</h3>
            <pre className="text-xs text-foreground/80 whitespace-pre-wrap leading-relaxed bg-secondary/30 rounded-lg p-4">
              {selectedSkill.content}
            </pre>
          </div>
        </div>
      </div>
    )
  }

  // ── Skills List View ───────────────────────────────────────────
  return (
    <div className="flex-1 min-h-0 overflow-y-auto p-6">
      <h2 className="text-lg font-semibold text-foreground mb-1">
        {t("settings.skills")}
      </h2>
      <p className="text-xs text-muted-foreground mb-4">
        {t("settings.skillsDesc")}
      </p>

      {/* Skill directories */}
      <div className="mb-5">
        <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">
          {t("settings.skillsDirs")}
        </h3>
        <div className="space-y-1">
          {/* Default directory (clickable) */}
          <button
            className="flex items-center gap-2 px-3 py-2 rounded-lg bg-secondary/30 text-xs w-full text-left hover:bg-secondary/50 transition-colors"
            onClick={() => handleOpenDir("~/.opencomputer/skills/")}
          >
            <FolderOpen className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
            <code className="flex-1 text-foreground/80 truncate">~/.opencomputer/skills/</code>
            <span className="text-[10px] px-1.5 py-0.5 rounded bg-secondary text-muted-foreground font-medium shrink-0">
              {t("settings.skillsDirDefault")}
            </span>
          </button>

          {/* Extra directories (clickable) */}
          {extraDirs.map((dir) => (
            <div key={dir} className="flex items-center gap-2 px-3 py-2 rounded-lg bg-secondary/30 text-xs group">
              <button
                className="flex items-center gap-2 flex-1 min-w-0 text-left hover:text-foreground transition-colors"
                onClick={() => handleOpenDir(dir)}
              >
                <FolderOpen className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
                <code className="flex-1 text-foreground/80 truncate" title={dir}>{dir}</code>
              </button>
              <button
                className="text-muted-foreground/50 hover:text-destructive transition-colors shrink-0 opacity-0 group-hover:opacity-100"
                onClick={() => handleRemoveDir(dir)}
                title={t("settings.skillsDirRemove")}
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </div>
          ))}
        </div>

        {/* Import directory button */}
        <button
          className="mt-2 flex items-center gap-1.5 text-xs text-primary hover:text-primary/80 transition-colors px-3 py-1.5"
          onClick={handleAddDir}
        >
          <FolderOpen className="h-3.5 w-3.5" />
          <span>{t("settings.skillsDirAdd")}</span>
        </button>
      </div>

      {/* Divider */}
      <div className="border-t border-border mb-4" />

      {/* Skills list */}
      <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">
        {t("settings.skillsList")}
        {!loading && skills.length > 0 && (
          <span className="ml-1.5 text-muted-foreground/60 font-normal normal-case">({skills.length})</span>
        )}
      </h3>

      {/* Env check toggle */}
      <div className="flex items-center justify-between px-1 mb-5">
        <div>
          <div className="text-sm text-foreground">{t("settings.agentSkillEnvCheck")}</div>
          <div className="text-xs text-muted-foreground">{t("settings.agentSkillEnvCheckDesc")}</div>
        </div>
        <Switch
          checked={skillEnvCheck}
          onCheckedChange={async (v) => {
            setSkillEnvCheck(v)
            await invoke("set_skill_env_check", { enabled: v })
          }}
        />
      </div>

      <div className="border-t border-border mb-4" />

      {loading ? (
        <div className="flex items-center justify-center py-12">
          <div className="animate-spin h-5 w-5 border-2 border-foreground border-t-transparent rounded-full" />
        </div>
      ) : skills.length === 0 ? (
        <div className="text-center py-12">
          <Puzzle className="h-10 w-10 text-muted-foreground/30 mx-auto mb-3" />
          <p className="text-sm text-muted-foreground">{t("settings.noSkills")}</p>
          <p className="text-xs text-muted-foreground/70 mt-1">{t("settings.noSkillsHint")}</p>
        </div>
      ) : (
        <div className="space-y-1">
          {skills.map((skill) => (
            <div
              key={skill.name}
              className={cn(
                "flex items-center gap-3 w-full px-3 py-2.5 rounded-lg text-sm transition-colors group",
                skill.enabled
                  ? "text-foreground hover:bg-secondary/60"
                  : "text-muted-foreground/50 hover:bg-secondary/40"
              )}
            >
              {/* Toggle */}
              <Switch
                checked={skill.enabled}
                onCheckedChange={(v) => handleToggleSkill(skill.name, v)}
                onClick={(e) => e.stopPropagation()}
              />

              {/* Name + description (clickable → detail) */}
              <button
                className="flex-1 text-left min-w-0"
                onClick={() => handleSelectSkill(skill.name)}
              >
                <div className={cn("font-medium truncate", !skill.enabled && "line-through")}>{skill.name}</div>
                <div className="text-xs text-muted-foreground truncate">{skill.description}</div>
              </button>

              {/* Source tag */}
              <span className="text-[10px] px-1.5 py-0.5 rounded bg-secondary text-muted-foreground font-medium shrink-0">
                {skill.source}
              </span>

              {/* Open directory */}
              <button
                className="shrink-0 text-muted-foreground/40 hover:text-muted-foreground transition-colors opacity-0 group-hover:opacity-100"
                onClick={(e) => { e.stopPropagation(); handleOpenDir(skill.base_dir) }}
                title={skill.base_dir}
              >
                <FolderOpen className="h-3.5 w-3.5" />
              </button>

              <ChevronRight
                className="h-4 w-4 text-muted-foreground/30 shrink-0 group-hover:text-muted-foreground/60 transition-colors cursor-pointer"
                onClick={() => handleSelectSkill(skill.name)}
              />
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

// ── Agent Management Panel ───────────────────────────────────────

interface AgentSummary {
  id: string
  name: string
  description?: string | null
  emoji?: string | null
  avatar?: string | null
  hasAgentMd: boolean
  hasPersona: boolean
  hasToolsGuide: boolean
}

interface PersonalityConfig {
  role?: string | null
  vibe?: string | null
  tone?: string | null
  traits: string[]
  principles: string[]
  boundaries?: string | null
  quirks?: string | null
  communicationStyle?: string | null
}

interface AgentConfig {
  name: string
  description?: string | null
  emoji?: string | null
  avatar?: string | null
  model: { primary?: string | null; fallbacks: string[] }
  skills: { allow: string[]; deny: string[] }
  tools: { allow: string[]; deny: string[] }
  personality: PersonalityConfig
  behavior: { maxToolRounds: number; requireApproval: string[]; sandbox: boolean; skillEnvCheck: boolean }
  useCustomPrompt: boolean
}

const DEFAULT_PERSONALITY: PersonalityConfig = {
  role: null,
  vibe: null,
  tone: null,
  traits: [],
  principles: [],
  boundaries: null,
  quirks: null,
  communicationStyle: null,
}

const TONE_PRESETS = [
  { value: "formal", labelKey: "settings.agentToneFormal" },
  { value: "casual", labelKey: "settings.agentToneCasual" },
  { value: "playful", labelKey: "settings.agentTonePlayful" },
  { value: "professional", labelKey: "settings.agentToneProfessional" },
  { value: "warm", labelKey: "settings.agentToneWarm" },
  { value: "direct", labelKey: "settings.agentToneDirect" },
]

function AgentPanel({ initialAgentId }: { initialAgentId?: string }) {
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
      console.error("Failed to load agents:", e)
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

type AgentTab = "identity" | "personality" | "behavior" | "model" | "custom"

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
        setConfig(cfg)
        setAgentMd(md ?? "")
        setPersona(per ?? "")
        setToolsGuide(tg ?? "")
        // Flag: content came from disk empty, will be filled with template after render
        if (!md) setNeedsFillTemplate(true)
      } catch (e) {
        console.error("Failed to load agent:", e)
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
      console.error("Failed to save agent:", e)
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
      console.error("Failed to delete agent:", e)
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
      console.error("Failed to pick avatar:", e)
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
      console.error("Failed to save avatar:", e)
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

  /** Generate template text from current structured config */
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
                  className="bg-secondary/40 rounded-lg resize-none leading-relaxed"
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
                  className="bg-secondary/40 rounded-lg resize-none leading-relaxed"
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
                  className="bg-secondary/40 rounded-lg resize-none leading-relaxed font-mono"
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
                  className="bg-secondary/40 rounded-lg resize-none leading-relaxed"
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
                  className="bg-secondary/40 rounded-lg resize-none leading-relaxed"
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
                  className="bg-secondary/40 rounded-lg resize-none leading-relaxed"
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
                  className="bg-secondary/40 rounded-lg resize-none leading-relaxed"
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
                  className="bg-secondary/40 rounded-lg resize-none leading-relaxed"
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
                  className="bg-secondary/40 rounded-lg resize-none leading-relaxed"
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
                  className="bg-secondary/40 rounded-lg resize-none leading-relaxed font-mono"
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
                  className="bg-secondary/40 rounded-lg resize-none leading-relaxed font-mono"
                  rows={5}
                  {...textInputProps(toolsGuide, setToolsGuide)}
                  placeholder={t("settings.agentToolsGuidePlaceholder")}
                />
                <CharCounter value={toolsGuide} />
              </div>
            </div>
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
                      className="bg-secondary/40 rounded-lg resize-none leading-relaxed font-mono"
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
                      className="bg-secondary/40 rounded-lg resize-none leading-relaxed font-mono"
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

// ── User Profile Panel ───────────────────────────────────────────

interface UserConfig {
  name?: string | null
  avatar?: string | null
  gender?: string | null
  age?: number | null
  role?: string | null
  timezone?: string | null
  language?: string | null
  aiExperience?: string | null
  responseStyle?: string | null
  customInfo?: string | null
}

const GENDER_PRESETS = ["male", "female"]

// Common timezones grouped by region, with i18n display names
const TIMEZONE_OPTIONS: { groupKey: string; zones: { value: string; labelKey: string }[] }[] = [
  { groupKey: "Asia", zones: [
    { value: "Asia/Shanghai", labelKey: "tz.shanghai" },
    { value: "Asia/Tokyo", labelKey: "tz.tokyo" },
    { value: "Asia/Seoul", labelKey: "tz.seoul" },
    { value: "Asia/Singapore", labelKey: "tz.singapore" },
    { value: "Asia/Hong_Kong", labelKey: "tz.hongkong" },
    { value: "Asia/Taipei", labelKey: "tz.taipei" },
    { value: "Asia/Kolkata", labelKey: "tz.kolkata" },
    { value: "Asia/Dubai", labelKey: "tz.dubai" },
    { value: "Asia/Bangkok", labelKey: "tz.bangkok" },
  ]},
  { groupKey: "Americas", zones: [
    { value: "America/New_York", labelKey: "tz.newyork" },
    { value: "America/Chicago", labelKey: "tz.chicago" },
    { value: "America/Denver", labelKey: "tz.denver" },
    { value: "America/Los_Angeles", labelKey: "tz.losangeles" },
    { value: "America/Toronto", labelKey: "tz.toronto" },
    { value: "America/Sao_Paulo", labelKey: "tz.saopaulo" },
    { value: "America/Mexico_City", labelKey: "tz.mexicocity" },
  ]},
  { groupKey: "Europe", zones: [
    { value: "Europe/London", labelKey: "tz.london" },
    { value: "Europe/Paris", labelKey: "tz.paris" },
    { value: "Europe/Berlin", labelKey: "tz.berlin" },
    { value: "Europe/Moscow", labelKey: "tz.moscow" },
    { value: "Europe/Istanbul", labelKey: "tz.istanbul" },
    { value: "Europe/Amsterdam", labelKey: "tz.amsterdam" },
    { value: "Europe/Madrid", labelKey: "tz.madrid" },
  ]},
  { groupKey: "Pacific", zones: [
    { value: "Pacific/Auckland", labelKey: "tz.auckland" },
    { value: "Australia/Sydney", labelKey: "tz.sydney" },
    { value: "Australia/Melbourne", labelKey: "tz.melbourne" },
    { value: "Pacific/Honolulu", labelKey: "tz.honolulu" },
  ]},
  { groupKey: "Other", zones: [
    { value: "UTC", labelKey: "tz.utc" },
  ]},
]

const LANGUAGE_OPTIONS = [
  { code: "zh-CN", label: "简体中文" },
  { code: "zh-TW", label: "繁體中文" },
  { code: "en", label: "English" },
  { code: "ja", label: "日本語" },
  { code: "ko", label: "한국어" },
  { code: "es", label: "Español" },
  { code: "pt", label: "Português" },
  { code: "ru", label: "Русский" },
  { code: "ar", label: "العربية" },
  { code: "tr", label: "Türkçe" },
  { code: "vi", label: "Tiếng Việt" },
  { code: "ms", label: "Bahasa Melayu" },
]

const PRESET_STYLES = ["concise", "detailed"]

function UserProfilePanel() {
  const { t, i18n } = useTranslation()
  const [config, setConfig] = useState<UserConfig>({})
  const [saving, setSaving] = useState(false)
  const [saved, setSaved] = useState(false)
  const [customStyle, setCustomStyle] = useState(false)
  const [customGender, setCustomGender] = useState(false)
  const composingRef = useRef(false)
  const [cropSrc, setCropSrc] = useState<string | null>(null)

  useEffect(() => {
    Promise.all([
      invoke<UserConfig>("get_user_config"),
      invoke<string>("get_system_timezone").catch(() => "UTC"),
    ]).then(([cfg, sysTz]) => {
      if (!cfg.timezone) cfg.timezone = sysTz
      if (!cfg.language) {
        const matched = LANGUAGE_OPTIONS.find((l) => i18n.language.startsWith(l.code))
        if (matched) cfg.language = matched.code
      }
      setConfig(cfg)
      if (cfg.responseStyle && !PRESET_STYLES.includes(cfg.responseStyle)) {
        setCustomStyle(true)
      }
      if (cfg.gender && !GENDER_PRESETS.includes(cfg.gender)) {
        setCustomGender(true)
      }
    }).catch(console.error)
  }, [i18n.language])

  const handleSave = async () => {
    setSaving(true)
    try {
      await invoke("save_user_config", { config })
      setSaved(true)
      setTimeout(() => setSaved(false), 2000)
    } catch (e) {
      console.error(e)
    } finally {
      setSaving(false)
    }
  }

  const update = (patch: Partial<UserConfig>) => {
    setConfig((prev) => ({ ...prev, ...patch }))
  }

  /** Props for text inputs that handle IME composition correctly */
  const textInputProps = (field: keyof UserConfig) => ({
    value: (config[field] as string) ?? "",
    onChange: (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => {
      // During IME composing, keep raw value; on blur, normalize empty to null
      update({ [field]: e.target.value })
    },
    onCompositionStart: () => { composingRef.current = true },
    onCompositionEnd: (e: React.CompositionEvent<HTMLInputElement | HTMLTextAreaElement>) => {
      composingRef.current = false
      update({ [field]: (e.target as HTMLInputElement).value })
    },
    onBlur: (e: React.FocusEvent<HTMLInputElement | HTMLTextAreaElement>) => {
      // Normalize: empty string → null for clean storage
      if (!e.target.value) update({ [field]: null })
    },
  })

  const handleAvatarPick = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog")
      const selected = await open({
        filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "gif", "webp", "svg"] }],
        multiple: false,
      })
      if (selected) {
        setCropSrc(convertFileSrc(selected as string))
      }
    } catch (e) {
      console.error("Failed to pick avatar:", e)
    }
  }

  const handleCropConfirm = async (blob: Blob) => {
    setCropSrc(null)
    try {
      const buf = await blob.arrayBuffer()
      const bytes = new Uint8Array(buf)
      let binary = ""
      for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i])
      const base64 = window.btoa(binary)
      const fileName = `user_${Date.now()}.png`
      const savedPath = await invoke<string>("save_avatar", { imageData: base64, fileName })
      update({ avatar: savedPath })
    } catch (e) {
      console.error("Failed to save avatar:", e)
    }
  }

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-hidden">
      {/* Scrollable content */}
      <div className="flex-1 overflow-y-auto p-6">
        <div className="max-w-4xl">
          <h2 className="text-lg font-semibold text-foreground mb-1">
            {t("settings.profile")}
          </h2>
          <p className="text-xs text-muted-foreground mb-5">
            {t("settings.profileDesc")}
          </p>

          <div className="space-y-5">

            {/* ── Avatar ── */}
            <div
              className="flex flex-col items-center gap-2 py-4 cursor-pointer"
              onClick={handleAvatarPick}
            >
              <div className="w-16 h-16 rounded-full bg-secondary border border-border/50 flex items-center justify-center overflow-hidden hover:border-primary/30 transition-colors">
                {config.avatar ? (
                  <img src={config.avatar.startsWith("/") ? convertFileSrc(config.avatar) : config.avatar} className="w-full h-full object-cover" alt="" />
                ) : (
                  <Camera className="h-5 w-5 text-muted-foreground/40" />
                )}
              </div>
              <span className="text-xs text-muted-foreground">{t("settings.profileAvatarChange")}</span>
            </div>

            {/* Avatar crop dialog */}
            {cropSrc && (
              <AvatarCropDialog
                open={!!cropSrc}
                imageSrc={cropSrc}
                onConfirm={handleCropConfirm}
                onCancel={() => setCropSrc(null)}
              />
            )}

            {/* ── Name ── */}
            <div>
              <div className="text-xs font-medium text-muted-foreground mb-2 px-1">{t("settings.profileName")}</div>
              <Input
                className="bg-secondary/40 rounded-lg"
                {...textInputProps("name")}
                placeholder={t("settings.profileNamePlaceholder")}
              />
            </div>

            {/* ── Gender ── */}
            <div>
              <div className="text-xs font-medium text-muted-foreground mb-2 px-1">{t("settings.profileGender")}</div>
              <div className="space-y-0.5">
                {GENDER_PRESETS.map((g) => (
                  <button
                    key={g}
                    className={cn(
                      "flex items-center gap-3 w-full px-3 py-2.5 rounded-lg text-sm transition-colors",
                      !customGender && config.gender === g
                        ? "bg-primary/10 text-primary font-medium"
                        : "bg-secondary/20 text-foreground hover:bg-secondary/60"
                    )}
                    onClick={() => {
                      setCustomGender(false)
                      update({ gender: config.gender === g ? null : g })
                    }}
                  >
                    <span className="flex-1 text-left">
                      {t(`settings.profileGender${g.charAt(0).toUpperCase() + g.slice(1)}`)}
                    </span>
                    {!customGender && config.gender === g && (
                      <Check className="h-4 w-4 text-primary shrink-0" />
                    )}
                  </button>
                ))}
                <button
                  className={cn(
                    "flex items-center gap-3 w-full px-3 py-2.5 rounded-lg text-sm transition-colors",
                    customGender
                      ? "bg-primary/10 text-primary font-medium"
                      : "bg-secondary/20 text-foreground hover:bg-secondary/60"
                  )}
                  onClick={() => {
                    setCustomGender(true)
                    if (!customGender) update({ gender: "" })
                  }}
                >
                  <span className="flex-1 text-left">{t("settings.profileGenderCustom")}</span>
                  {customGender && <Check className="h-4 w-4 text-primary shrink-0" />}
                </button>
              </div>
              {customGender && (
                <Input
                  className="mt-2 bg-secondary/40 rounded-lg"
                  {...textInputProps("gender")}
                  placeholder={t("settings.profileGenderCustomPlaceholder")}
                />
              )}
            </div>

            {/* ── Age ── */}
            <div>
              <div className="text-xs font-medium text-muted-foreground mb-2 px-1">{t("settings.profileAge")}</div>
              <Input
                type="number"
                min={1}
                max={150}
                className="bg-secondary/40 rounded-lg [appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none [&::-webkit-outer-spin-button]:appearance-none"
                value={config.age ?? ""}
                onChange={(e) => {
                  const v = e.target.value
                  update({ age: v ? parseInt(v, 10) || null : null })
                }}
                placeholder={t("settings.profileAgePlaceholder")}
              />
            </div>

            {/* ── Role ── */}
            <div>
              <div className="text-xs font-medium text-muted-foreground mb-2 px-1">{t("settings.profileRole")}</div>
              <Input
                className="bg-secondary/40 rounded-lg"
                {...textInputProps("role")}
                placeholder={t("settings.profileRolePlaceholder")}
              />
            </div>

            {/* ── AI Experience ── */}
            <div>
              <div className="text-xs font-medium text-muted-foreground mb-2 px-1">{t("settings.profileAiExperience")}</div>
              <div className="space-y-0.5">
                {(["expert", "intermediate", "beginner"] as const).map((level) => (
                  <button
                    key={level}
                    className={cn(
                      "flex items-center gap-3 w-full px-3 py-2.5 rounded-lg text-sm transition-colors",
                      config.aiExperience === level
                        ? "bg-primary/10 text-primary font-medium"
                        : "bg-secondary/20 text-foreground hover:bg-secondary/60"
                    )}
                    onClick={() => update({ aiExperience: config.aiExperience === level ? null : level })}
                  >
                    <span className="flex-1 text-left">
                      {t(`settings.profileAiExp${level.charAt(0).toUpperCase() + level.slice(1)}`)}
                    </span>
                    {config.aiExperience === level && (
                      <Check className="h-4 w-4 text-primary shrink-0" />
                    )}
                  </button>
                ))}
              </div>
            </div>

            <div className="border-t border-border/50" />

            {/* ── Timezone ── */}
            <div>
              <div className="text-xs font-medium text-muted-foreground mb-2 px-1">{t("settings.profileTimezone")}</div>
              <div className="space-y-0.5">
                <button
                  className={cn(
                    "flex items-center gap-3 w-full px-3 py-2.5 rounded-lg text-sm transition-colors",
                    !config.timezone
                      ? "bg-primary/10 text-primary font-medium"
                      : "bg-secondary/20 text-foreground hover:bg-secondary/60"
                  )}
                  onClick={() => update({ timezone: null })}
                >
                  <Monitor className="h-4 w-4 shrink-0 opacity-60" />
                  <span className="flex-1 text-left">{t("settings.profileTimezoneSystem")}</span>
                  {!config.timezone && <Check className="h-4 w-4 text-primary shrink-0" />}
                </button>
              </div>
              <Select value={config.timezone ?? ""} onValueChange={(v) => update({ timezone: v || null })}>
                <SelectTrigger className="mt-1 bg-secondary/20 text-sm hover:bg-secondary/60">
                  <SelectValue placeholder={t("settings.profileTimezoneSystem")} />
                </SelectTrigger>
                <SelectContent>
                  {TIMEZONE_OPTIONS.map((group) => (
                    <SelectGroup key={group.groupKey}>
                      <SelectLabel>{group.groupKey}</SelectLabel>
                      {group.zones.map((tz) => (
                        <SelectItem key={tz.value} value={tz.value}>{t(tz.labelKey)}</SelectItem>
                      ))}
                    </SelectGroup>
                  ))}
                </SelectContent>
              </Select>
            </div>

            {/* ── Language ── */}
            <div>
              <div className="text-xs font-medium text-muted-foreground mb-2 px-1">{t("settings.profileLanguage")}</div>
              <div className="space-y-0.5">
                <button
                  className={cn(
                    "flex items-center gap-3 w-full px-3 py-2.5 rounded-lg text-sm transition-colors",
                    !config.language
                      ? "bg-primary/10 text-primary font-medium"
                      : "bg-secondary/20 text-foreground hover:bg-secondary/60"
                  )}
                  onClick={() => update({ language: null })}
                >
                  <Monitor className="h-4 w-4 shrink-0 opacity-60" />
                  <span className="flex-1 text-left">{t("settings.profileLanguageSystem")}</span>
                  {!config.language && <Check className="h-4 w-4 text-primary shrink-0" />}
                </button>
              </div>
              <Select value={config.language ?? ""} onValueChange={(v) => update({ language: v || null })}>
                <SelectTrigger className="mt-1 bg-secondary/20 text-sm hover:bg-secondary/60">
                  <SelectValue placeholder={t("settings.profileLanguageSystem")} />
                </SelectTrigger>
                <SelectContent>
                  {LANGUAGE_OPTIONS.map((lang) => (
                    <SelectItem key={lang.code} value={lang.code}>{lang.label}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <div className="border-t border-border/50" />

            {/* ── Response Style ── */}
            <div>
              <div className="text-xs font-medium text-muted-foreground mb-2 px-1">{t("settings.profileResponseStyle")}</div>
              <div className="space-y-0.5">
                {PRESET_STYLES.map((style) => (
                  <button
                    key={style}
                    className={cn(
                      "flex items-center gap-3 w-full px-3 py-2.5 rounded-lg text-sm transition-colors",
                      !customStyle && config.responseStyle === style
                        ? "bg-primary/10 text-primary font-medium"
                        : "bg-secondary/20 text-foreground hover:bg-secondary/60"
                    )}
                    onClick={() => {
                      setCustomStyle(false)
                      update({ responseStyle: config.responseStyle === style ? null : style })
                    }}
                  >
                    <span className="flex-1 text-left">
                      {t(`settings.profileStyle${style.charAt(0).toUpperCase() + style.slice(1)}`)}
                    </span>
                    {!customStyle && config.responseStyle === style && (
                      <Check className="h-4 w-4 text-primary shrink-0" />
                    )}
                  </button>
                ))}
                <button
                  className={cn(
                    "flex items-center gap-3 w-full px-3 py-2.5 rounded-lg text-sm transition-colors",
                    customStyle
                      ? "bg-primary/10 text-primary font-medium"
                      : "bg-secondary/20 text-foreground hover:bg-secondary/60"
                  )}
                  onClick={() => {
                    setCustomStyle(true)
                    if (!customStyle) update({ responseStyle: "" })
                  }}
                >
                  <span className="flex-1 text-left">{t("settings.profileStyleCustom")}</span>
                  {customStyle && <Check className="h-4 w-4 text-primary shrink-0" />}
                </button>
              </div>

              {customStyle && (
                <Textarea
                  className="mt-2 bg-secondary/40 rounded-lg resize-none leading-relaxed"
                  rows={4}
                  {...textInputProps("responseStyle")}
                  placeholder={t("settings.profileStyleCustomPlaceholder")}
                />
              )}
            </div>

            <div className="border-t border-border/50" />

            {/* ── Custom Info ── */}
            <div>
              <div className="text-xs font-medium text-muted-foreground mb-2 px-1">{t("settings.profileCustomInfo")}</div>
              <Textarea
                className="bg-secondary/40 rounded-lg resize-none leading-relaxed"
                rows={5}
                {...textInputProps("customInfo")}
                placeholder={t("settings.profileCustomInfoPlaceholder")}
              />
            </div>

          </div>
        </div>
      </div>

      {/* ── Save — fixed bottom-right ── */}
      <div className="shrink-0 flex justify-end px-6 py-3 border-t border-border/30">
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
              {t("settings.profileSaved")}
            </span>
          ) : t("common.save")}
        </Button>
      </div>
    </div>
  )
}

// ── About Panel ───────────────────────────────────────────────────

function AboutPanel() {
  const { t } = useTranslation()

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="flex flex-col items-center text-center py-8 max-w-xl mx-auto">
        {/* App Icon */}
        <div className="w-20 h-20 rounded-2xl bg-gradient-to-br from-primary/20 via-primary/10 to-transparent border border-border/50 flex items-center justify-center mb-5 shadow-lg">
          <span className="text-3xl font-bold text-primary">OC</span>
        </div>

        <h2 className="text-xl font-bold text-foreground mb-1">
          OpenComputer
        </h2>
        <p className="text-xs text-muted-foreground mb-4">
          {t("about.version")} 0.1.0
        </p>

        <p className="text-sm text-muted-foreground leading-relaxed max-w-sm mb-6">
          {t("about.description")}
        </p>

        <div className="flex items-center gap-4">
          <a
            href="https://github.com"
            target="_blank"
            rel="noreferrer"
            className="text-xs text-muted-foreground hover:text-foreground transition-colors underline underline-offset-2"
          >
            {t("about.github")}
          </a>
        </div>
      </div>

      {/* Tech Stack */}
      <div className="border-t border-border pt-5 mt-2 max-w-xl mx-auto">
        <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-3">
          {t("about.techStack")}
        </h3>
        <div className="grid grid-cols-2 gap-2 text-xs">
          {[
            ["Frontend", "React 19 + TypeScript"],
            ["Backend", "Rust + Tauri 2"],
            ["Styling", "Tailwind CSS v4"],
            ["UI", "shadcn/ui (Radix)"],
          ].map(([label, value]) => (
            <div
              key={label}
              className="flex flex-col gap-0.5 bg-secondary/40 rounded-lg px-3 py-2 border border-border/30"
            >
              <span className="text-muted-foreground">{label}</span>
              <span className="text-foreground font-medium">{value}</span>
            </div>
          ))}
        </div>
      </div>
    </div>
  )
}

// ── Main SettingsView Component ───────────────────────────────────

export default function SettingsView({
  onBack,
  onCodexAuth,
  onCodexReauth,
  initialSection,
  initialAgentId,
}: {
  onBack: () => void
  onCodexAuth: () => Promise<void>
  onCodexReauth?: () => void
  initialSection?: SettingsSection
  initialAgentId?: string
}) {
  const { t } = useTranslation()
  const [activeSection, setActiveSection] =
    useState<SettingsSection>(initialSection ?? "providers")
  const [addingProvider, setAddingProvider] = useState(false)
  const [editingProvider, setEditingProvider] = useState<ProviderConfig | null>(null)

  return (
    <div className="flex flex-1 h-full overflow-hidden bg-background">
      {/* Left Sidebar — Settings Navigation */}
      <div className="w-[220px] shrink-0 border-r border-border bg-secondary/20 flex flex-col">
        {/* Header with back button + drag region */}
        <div className="h-10 flex items-end px-4 gap-2 shrink-0" data-tauri-drag-region>
          <Button
            variant="ghost"
            size="sm"
            onClick={onBack}
            className="gap-1.5 text-muted-foreground hover:text-foreground pb-1.5"
          >
            <ArrowLeft className="h-4 w-4" />
            <span className="text-sm font-semibold text-foreground">
              {t("settings.title")}
            </span>
          </Button>
        </div>

        {/* Navigation Items */}
        <div className="flex-1 overflow-y-auto p-2 space-y-0.5">
          {SECTIONS.map((section) => (
            <button
              key={section.id}
              className={cn(
                "flex items-center gap-2.5 w-full px-3 py-2 rounded-lg text-sm transition-all duration-150",
                activeSection === section.id
                  ? "bg-secondary text-foreground font-medium shadow-sm"
                  : "text-muted-foreground hover:bg-secondary/60 hover:text-foreground"
              )}
              onClick={() => setActiveSection(section.id)}
            >
              <span
                className={cn(
                  "shrink-0",
                  activeSection === section.id
                    ? "text-primary"
                    : "text-muted-foreground"
                )}
              >
                {section.icon}
              </span>
              {t(section.labelKey)}
            </button>
          ))}
        </div>
      </div>

      {/* Right Content Panel */}
      <div className="flex-1 flex flex-col min-w-0 overflow-hidden">
        {/* Content Header + drag region */}
        <div className="h-10 flex items-end px-6 shrink-0" data-tauri-drag-region>
          <span className="text-sm font-semibold text-foreground pb-1.5">
            {t(
              SECTIONS.find((s) => s.id === activeSection)?.labelKey ??
                "settings.title"
            )}
          </span>
        </div>

        {/* Content Area */}
        <div className="flex-1 flex flex-col min-h-0 overflow-hidden">
          {activeSection === "providers" && (
            addingProvider ? (
              <ProviderSetup
                onComplete={() => setAddingProvider(false)}
                onCodexAuth={onCodexAuth}
                onCancel={() => setAddingProvider(false)}
              />
            ) : editingProvider ? (
              <ProviderEditPage
                provider={editingProvider}
                onSave={() => setEditingProvider(null)}
                onCancel={() => setEditingProvider(null)}
                onCodexReauth={onCodexReauth}
              />
            ) : (
              <ProviderSettings
                onAddProvider={() => setAddingProvider(true)}
                onEditProvider={(p) => setEditingProvider(p)}
                onCodexReauth={onCodexReauth}
              />
            )
          )}
          {activeSection === "models" && <GlobalModelPanel />}
          {activeSection === "skills" && <SkillsPanel />}
          {activeSection === "agents" && <AgentPanel initialAgentId={initialAgentId} />}
          {activeSection === "profile" && <UserProfilePanel />}
          {activeSection === "chat" && <ChatSettingsPanel />}
          {activeSection === "appearance" && <AppearancePanel />}
          {activeSection === "language" && <LanguagePanel />}
          {activeSection === "about" && <AboutPanel />}
        </div>
      </div>
    </div>
  )
}
