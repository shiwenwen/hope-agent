import { useState, useEffect, useCallback } from "react"
import { invoke, Channel } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import i18n from "@/i18n/i18n"
import { SUPPORTED_LANGUAGES } from "@/i18n/i18n"
import { logger } from "@/lib/logger"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { TooltipProvider, IconTip } from "@/components/ui/tooltip"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
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
  Check,
  ChevronDown,
  ChevronRight,
  Circle,
  Download,
  ExternalLink,
  GripVertical,
  Loader2,
  Play,
  RefreshCw,
  Square,
  Trash2,
} from "lucide-react"

// ── Types ────────────────────────────────────────────────────────

interface ProviderEntry {
  id: string
  enabled: boolean
  apiKey: string | null
  apiKey2: string | null
  baseUrl: string | null
}

interface WebSearchConfig {
  providers: ProviderEntry[]
  searxngDockerManaged: boolean | null
  defaultResultCount: number
  timeoutSeconds: number
  cacheTtlMinutes: number
  defaultCountry: string | null
  defaultLanguage: string | null
  defaultFreshness: string | null
}

interface SearxngDockerStatus {
  dockerInstalled: boolean
  dockerNotRunning: boolean
  containerExists: boolean
  containerRunning: boolean
  port: number | null
  healthOk: boolean
}

// ── Provider metadata (static) ──────────────────────────────────

interface ProviderMeta {
  id: string
  labelKey: string
  free: boolean
  needsApiKey: boolean
  url: string
  fields: FieldDef[]
}

interface FieldDef {
  configKey: "apiKey" | "apiKey2" | "baseUrl"
  labelKey: string
  placeholder: string
  secret?: boolean
}

const PROVIDER_META: Record<string, ProviderMeta> = {
  "duck-duck-go": {
    id: "duck-duck-go",
    labelKey: "settings.webSearchProviderDDG",
    free: true,
    needsApiKey: false,
    url: "https://duckduckgo.com",
    fields: [],
  },
  searxng: {
    id: "searxng",
    labelKey: "settings.webSearchProviderSearXNG",
    free: true,
    needsApiKey: false,
    url: "https://docs.searxng.org",
    fields: [
      {
        configKey: "baseUrl",
        labelKey: "settings.webSearchInstanceUrl",
        placeholder: "http://localhost:8080",
      },
    ],
  },
  brave: {
    id: "brave",
    labelKey: "settings.webSearchProviderBrave",
    free: false,
    needsApiKey: true,
    url: "https://brave.com/search/api/",
    fields: [
      {
        configKey: "apiKey",
        labelKey: "settings.webSearchApiKey",
        placeholder: "BSA...",
        secret: true,
      },
    ],
  },
  perplexity: {
    id: "perplexity",
    labelKey: "settings.webSearchProviderPerplexity",
    free: false,
    needsApiKey: true,
    url: "https://docs.perplexity.ai",
    fields: [
      {
        configKey: "apiKey",
        labelKey: "settings.webSearchApiKey",
        placeholder: "pplx-...",
        secret: true,
      },
    ],
  },
  google: {
    id: "google",
    labelKey: "settings.webSearchProviderGoogle",
    free: false,
    needsApiKey: true,
    url: "https://developers.google.com/custom-search/v1/overview",
    fields: [
      {
        configKey: "apiKey",
        labelKey: "settings.webSearchApiKey",
        placeholder: "AIza...",
        secret: true,
      },
      {
        configKey: "apiKey2",
        labelKey: "settings.webSearchGoogleCx",
        placeholder: "Search Engine ID",
      },
    ],
  },
  grok: {
    id: "grok",
    labelKey: "settings.webSearchProviderGrok",
    free: false,
    needsApiKey: true,
    url: "https://console.x.ai",
    fields: [
      {
        configKey: "apiKey",
        labelKey: "settings.webSearchApiKey",
        placeholder: "xai-...",
        secret: true,
      },
    ],
  },
  kimi: {
    id: "kimi",
    labelKey: "settings.webSearchProviderKimi",
    free: false,
    needsApiKey: true,
    url: "https://platform.moonshot.cn",
    fields: [
      {
        configKey: "apiKey",
        labelKey: "settings.webSearchApiKey",
        placeholder: "sk-...",
        secret: true,
      },
    ],
  },
  tavily: {
    id: "tavily",
    labelKey: "settings.webSearchProviderTavily",
    free: false,
    needsApiKey: true,
    url: "https://tavily.com",
    fields: [
      {
        configKey: "apiKey",
        labelKey: "settings.webSearchApiKey",
        placeholder: "tvly-...",
        secret: true,
      },
    ],
  },
}

function hasRequiredCredentials(entry: ProviderEntry): boolean {
  const meta = PROVIDER_META[entry.id]
  if (!meta) return false
  // DuckDuckGo: always ready
  if (entry.id === "duck-duck-go") return true
  // SearXNG: needs baseUrl (instance address)
  if (entry.id === "searxng") return !!entry.baseUrl?.trim()
  // Paid providers: need apiKey
  if (!entry.apiKey?.trim()) return false
  // Google also needs apiKey2 (CX)
  if (entry.id === "google" && !entry.apiKey2?.trim()) return false
  return true
}

// ── Sortable Provider Row ───────────────────────────────────────

function SortableProviderItem({
  entry,
  index,
  expanded,
  onToggleExpand,
  onToggleEnabled,
  onFieldChange,
}: {
  entry: ProviderEntry
  index: number
  expanded: boolean
  onToggleExpand: () => void
  onToggleEnabled: (enabled: boolean) => void
  onFieldChange: (key: "apiKey" | "apiKey2" | "baseUrl", value: string | null) => void
}) {
  const { t } = useTranslation()
  const meta = PROVIDER_META[entry.id]
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: entry.id,
  })

  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.4 : 1,
    zIndex: isDragging ? 50 : undefined,
  }

  if (!meta) return null

  const canEnable = hasRequiredCredentials(entry)
  const hasFields = meta.fields.length > 0

  return (
    <div
      ref={setNodeRef}
      style={style}
      className="rounded-lg border border-border/50 bg-secondary/20 overflow-hidden"
    >
      {/* Main row */}
      <div className="flex items-center gap-2 px-3 py-2.5">
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

        {/* Expand toggle + name */}
        <button
          className="flex items-center gap-1.5 flex-1 min-w-0 text-left"
          onClick={onToggleExpand}
        >
          {hasFields ? (
            expanded ? (
              <ChevronDown className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
            ) : (
              <ChevronRight className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
            )
          ) : (
            <span className="w-3.5 shrink-0" />
          )}
          <span className="text-sm font-medium truncate">{t(meta.labelKey)}</span>
          {meta.free && (
            <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-green-500/10 text-green-600 dark:text-green-400 font-medium shrink-0">
              {t("settings.webSearchFree")}
            </span>
          )}
          {!canEnable && entry.id !== "duck-duck-go" && (
            <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-yellow-500/10 text-yellow-600 dark:text-yellow-400 font-medium shrink-0">
              {t(meta.needsApiKey ? "settings.webSearchNeedsKey" : "settings.webSearchNeedsConfig")}
            </span>
          )}
        </button>

        {/* Website link */}
        <button
          type="button"
          className="text-muted-foreground/40 hover:text-primary shrink-0 transition-colors"
          onClick={() => invoke("open_url", { url: meta.url })}
          title={meta.url}
        >
          <ExternalLink className="h-3.5 w-3.5" />
        </button>

        {/* Enable toggle */}
        <Switch
          checked={entry.enabled}
          disabled={!canEnable && !entry.enabled}
          onCheckedChange={onToggleEnabled}
        />
      </div>

      {/* Expanded fields */}
      {expanded && hasFields && (
        <div className="px-3 pb-3 pt-1 space-y-2.5 border-t border-border/30 ml-[52px]">
          {meta.fields.map((field) => (
            <div key={field.configKey} className="space-y-1">
              <label className="text-xs font-medium text-muted-foreground">
                {t(field.labelKey)}
              </label>
              <Input
                type={field.secret ? "password" : "text"}
                placeholder={field.placeholder}
                className="h-8 text-sm"
                value={(entry[field.configKey] as string) ?? ""}
                onChange={(e) => onFieldChange(field.configKey, e.target.value || null)}
              />
            </div>
          ))}

          {/* SearXNG Docker section */}
          {entry.id === "searxng" && (
            <SearxngDockerSection onUrlSet={(url) => onFieldChange("baseUrl", url)} />
          )}
        </div>
      )}
    </div>
  )
}

// ── SearXNG Docker Section ──────────────────────────────────────

function SearxngDockerSection({ onUrlSet }: { onUrlSet: (url: string) => void }) {
  const { t } = useTranslation()
  const [status, setStatus] = useState<SearxngDockerStatus | null>(null)
  const [checking, setChecking] = useState(true)
  const [deploying, setDeploying] = useState(false)
  const [deployStep, setDeployStep] = useState<string | null>(null)
  const [actionLoading, setActionLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const refreshStatus = useCallback(async () => {
    setChecking(true)
    try {
      const s = await invoke<SearxngDockerStatus>("searxng_docker_status")
      setStatus(s)
    } catch (e) {
      logger.error("settings", "SearxngDocker::status", "Failed to check Docker status", e)
    } finally {
      setChecking(false)
    }
  }, [])

  useEffect(() => {
    refreshStatus()
  }, [refreshStatus])

  // Poll status while container is running but not yet healthy
  useEffect(() => {
    if (!status?.containerRunning || status?.healthOk) return
    const timer = setInterval(async () => {
      try {
        const s = await invoke<SearxngDockerStatus>("searxng_docker_status")
        setStatus(s)
        if (s.healthOk) clearInterval(timer)
      } catch {
        /* ignore */
      }
    }, 3000)
    return () => clearInterval(timer)
  }, [status?.containerRunning, status?.healthOk])

  const deployStepLabels: Record<string, string> = {
    checking_docker: t("settings.webSearchDockerStepCheckingDocker"),
    pulling_image: t("settings.webSearchDockerStepPullingImage"),
    removing_old: t("settings.webSearchDockerStepRemovingOld"),
    starting_container: t("settings.webSearchDockerStepStarting"),
    injecting_config: t("settings.webSearchDockerStepConfig"),
    restarting: t("settings.webSearchDockerStepRestarting"),
    health_check: t("settings.webSearchDockerStepHealthCheck"),
    done: t("settings.webSearchDockerStepDone"),
  }

  const handleDeploy = useCallback(async () => {
    setDeploying(true)
    setDeployStep(null)
    setError(null)
    try {
      const channel = new Channel<string>()
      channel.onmessage = (step) => {
        setDeployStep(step)
      }
      const url = await invoke<string>("searxng_docker_deploy", { channel })
      onUrlSet(url)
      await refreshStatus()
    } catch (e) {
      setError(String(e))
    } finally {
      setDeploying(false)
      setDeployStep(null)
    }
  }, [onUrlSet, refreshStatus])

  const handleAction = useCallback(
    async (action: "start" | "stop" | "remove") => {
      setActionLoading(true)
      setError(null)
      try {
        await invoke(`searxng_docker_${action}`)
        await refreshStatus()
        // After start, poll until healthy (up to 15s)
        if (action === "start") {
          for (let i = 0; i < 10; i++) {
            await new Promise((r) => setTimeout(r, 1500))
            const s = await invoke<SearxngDockerStatus>("searxng_docker_status")
            setStatus(s)
            if (s.healthOk) break
          }
        }
      } catch (e) {
        setError(String(e))
      } finally {
        setActionLoading(false)
      }
    },
    [refreshStatus],
  )

  if (checking && !status) {
    return (
      <div className="rounded-md border border-border/50 p-3 mt-1">
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <Loader2 className="h-3.5 w-3.5 animate-spin" />
          {t("settings.webSearchDockerChecking")}
        </div>
      </div>
    )
  }

  if (!status) return null

  if (!status.dockerInstalled) {
    return (
      <div className="rounded-md border border-border/50 p-3 mt-1 space-y-2">
        <div className="text-xs font-medium">{t("settings.webSearchDockerTitle")}</div>
        <p className="text-xs text-muted-foreground">{t("settings.webSearchDockerNotInstalled")}</p>
        <Button
          size="sm"
          variant="outline"
          className="h-7 text-xs"
          onClick={() =>
            invoke("open_url", { url: "https://www.docker.com/products/docker-desktop/" })
          }
        >
          <ExternalLink className="h-3 w-3 mr-1" />
          {t("settings.webSearchDockerInstall")}
        </Button>
      </div>
    )
  }

  if (status.dockerNotRunning) {
    return (
      <div className="rounded-md border border-border/50 p-3 mt-1 space-y-2">
        <div className="text-xs font-medium">{t("settings.webSearchDockerTitle")}</div>
        <p className="text-xs text-muted-foreground">{t("settings.webSearchDockerNotRunning")}</p>
        <Button size="sm" variant="outline" className="h-7 text-xs" onClick={refreshStatus}>
          <RefreshCw className="h-3 w-3 mr-1" />
          {t("settings.webSearchDockerRefresh")}
        </Button>
      </div>
    )
  }

  return (
    <div className="rounded-md border border-border/50 p-3 mt-1 space-y-2">
      <div className="text-xs font-medium">{t("settings.webSearchDockerTitle")}</div>

      {status.containerExists && (
        <div className="flex items-center gap-2 text-xs">
          <Circle
            className={`h-2 w-2 fill-current ${
              status.containerRunning && status.healthOk
                ? "text-green-500"
                : status.containerRunning
                  ? "text-yellow-500"
                  : "text-muted-foreground"
            }`}
          />
          <span>
            {status.containerRunning
              ? status.healthOk
                ? t("settings.webSearchDockerRunning")
                : t("settings.webSearchDockerStarting")
              : t("settings.webSearchDockerStopped")}
          </span>
          {status.port && status.containerRunning && (
            <TooltipProvider>
              <IconTip label={t("settings.webSearchDockerFillUrl")}>
                <button
                  type="button"
                  className="text-muted-foreground hover:text-primary underline decoration-dotted underline-offset-2 transition-colors"
                  onClick={() => onUrlSet(`http://localhost:${status.port}`)}
                >
                  localhost:{status.port}
                </button>
              </IconTip>
            </TooltipProvider>
          )}
        </div>
      )}

      {error && <p className="text-xs text-destructive whitespace-pre-wrap break-all">{error}</p>}

      {deploying && deployStep && (
        <p className="text-xs text-muted-foreground">
          <Loader2 className="h-3 w-3 animate-spin inline mr-1" />
          {deployStepLabels[deployStep] || deployStep}
        </p>
      )}

      <div className="flex items-center gap-2">
        {!status.containerExists && (
          <Button
            size="sm"
            variant="outline"
            className="h-7 text-xs"
            onClick={handleDeploy}
            disabled={deploying}
          >
            {deploying ? (
              <Loader2 className="h-3 w-3 animate-spin mr-1" />
            ) : (
              <Download className="h-3 w-3 mr-1" />
            )}
            {deploying
              ? t("settings.webSearchDockerDeploying")
              : t("settings.webSearchDockerDeploy")}
          </Button>
        )}
        {status.containerExists && !status.containerRunning && (
          <Button
            size="sm"
            variant="outline"
            className="h-7 text-xs"
            onClick={() => handleAction("start")}
            disabled={actionLoading}
          >
            {actionLoading ? (
              <Loader2 className="h-3 w-3 animate-spin mr-1" />
            ) : (
              <Play className="h-3 w-3 mr-1" />
            )}
            {t("settings.webSearchDockerStart")}
          </Button>
        )}
        {status.containerExists && status.containerRunning && (
          <Button
            size="sm"
            variant="outline"
            className="h-7 text-xs"
            onClick={() => handleAction("stop")}
            disabled={actionLoading}
          >
            {actionLoading ? (
              <Loader2 className="h-3 w-3 animate-spin mr-1" />
            ) : (
              <Square className="h-3 w-3 mr-1" />
            )}
            {t("settings.webSearchDockerStop")}
          </Button>
        )}
        {status.containerExists && (
          <Button
            size="sm"
            variant="ghost"
            className="h-7 text-xs text-destructive hover:text-destructive"
            onClick={() => handleAction("remove")}
            disabled={actionLoading || deploying}
          >
            <Trash2 className="h-3 w-3 mr-1" />
            {t("settings.webSearchDockerRemove")}
          </Button>
        )}
      </div>
    </div>
  )
}

// ── Main Component ──────────────────────────────────────────────

export default function WebSearchPanel() {
  const { t } = useTranslation()
  const [config, setConfig] = useState<WebSearchConfig | null>(null)
  const [savedJson, setSavedJson] = useState("")
  const [saving, setSaving] = useState(false)
  const [justSaved, setJustSaved] = useState(false)
  const [expandedId, setExpandedId] = useState<string | null>(null)
  const [advancedOpen, setAdvancedOpen] = useState(true)

  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 5 } }))

  useEffect(() => {
    invoke<WebSearchConfig>("get_web_search_config")
      .then((cfg) => {
        setConfig(cfg)
        setSavedJson(JSON.stringify(cfg))
      })
      .catch((e) => logger.error("settings", "WebSearchPanel::load", "Failed to load config", e))
  }, [])

  const isDirty = config ? JSON.stringify(config) !== savedJson : false

  const handleSave = useCallback(async () => {
    if (!config) return
    setSaving(true)
    try {
      await invoke("save_web_search_config", { config })
      setSavedJson(JSON.stringify(config))
      setJustSaved(true)
      setTimeout(() => setJustSaved(false), 2000)
    } catch (e) {
      logger.error("settings", "WebSearchPanel::save", "Failed to save config", e)
    } finally {
      setSaving(false)
    }
  }, [config])

  const handleDragEnd = useCallback(
    (event: DragEndEvent) => {
      const { active, over } = event
      if (!over || !config || active.id === over.id) return
      const oldIndex = config.providers.findIndex((p) => p.id === active.id)
      const newIndex = config.providers.findIndex((p) => p.id === over.id)
      if (oldIndex === -1 || newIndex === -1) return
      setConfig((prev) =>
        prev ? { ...prev, providers: arrayMove(prev.providers, oldIndex, newIndex) } : prev,
      )
    },
    [config],
  )

  const handleToggleEnabled = useCallback((id: string, enabled: boolean) => {
    setConfig((prev) => {
      if (!prev) return prev
      return {
        ...prev,
        providers: prev.providers.map((p) => (p.id === id ? { ...p, enabled } : p)),
      }
    })
  }, [])

  const handleFieldChange = useCallback(
    (id: string, key: "apiKey" | "apiKey2" | "baseUrl", value: string | null) => {
      setConfig((prev) => {
        if (!prev) return prev
        const providers = prev.providers.map((p) => {
          if (p.id !== id) return p
          const updated = { ...p, [key]: value }
          // Auto-disable if key was cleared and provider requires key
          const meta = PROVIDER_META[id]
          if (meta?.needsApiKey && !hasRequiredCredentials(updated)) {
            updated.enabled = false
          }
          return updated
        })
        return { ...prev, providers }
      })
    },
    [],
  )

  if (!config) return null

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="space-y-4">
        <p className="text-xs text-muted-foreground">{t("settings.webSearchDesc")}</p>

        {/* Drag-sortable provider list */}
        <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={handleDragEnd}>
          <SortableContext
            items={config.providers.map((p) => p.id)}
            strategy={verticalListSortingStrategy}
          >
            <div className="space-y-2">
              {config.providers.map((entry, index) => (
                <SortableProviderItem
                  key={entry.id}
                  entry={entry}
                  index={index}
                  expanded={expandedId === entry.id}
                  onToggleExpand={() =>
                    setExpandedId((prev) => (prev === entry.id ? null : entry.id))
                  }
                  onToggleEnabled={(enabled) => handleToggleEnabled(entry.id, enabled)}
                  onFieldChange={(key, value) => handleFieldChange(entry.id, key, value)}
                />
              ))}
            </div>
          </SortableContext>
        </DndContext>

        {/* Advanced settings */}
        <div className="rounded-lg border border-border/50 bg-secondary/20 overflow-hidden">
          <button
            className="flex items-center gap-2 px-3 py-2.5 w-full text-left"
            onClick={() => setAdvancedOpen((prev) => !prev)}
          >
            {advancedOpen ? (
              <ChevronDown className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
            ) : (
              <ChevronRight className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
            )}
            <span className="text-sm font-medium">{t("settings.webSearchAdvanced")}</span>
          </button>

          {advancedOpen && (
            <div className="px-3 pb-3 pt-1 space-y-3 border-t border-border/30">
              <div className="grid grid-cols-3 gap-3">
                {/* Default result count */}
                <div className="space-y-1">
                  <label className="text-xs font-medium text-muted-foreground">
                    {t("settings.webSearchDefaultCount")}
                  </label>
                  <Input
                    type="number"
                    min={1}
                    max={10}
                    className="h-8 text-sm"
                    value={config.defaultResultCount}
                    onChange={(e) =>
                      setConfig((prev) =>
                        prev
                          ? {
                              ...prev,
                              defaultResultCount: Math.max(
                                1,
                                Math.min(10, Number(e.target.value) || 5),
                              ),
                            }
                          : prev,
                      )
                    }
                  />
                  <p className="text-[10px] text-muted-foreground/60">
                    {t("settings.webSearchDefaultCountDesc")}
                  </p>
                </div>

                {/* Timeout */}
                <div className="space-y-1">
                  <label className="text-xs font-medium text-muted-foreground">
                    {t("settings.webSearchTimeout")}
                  </label>
                  <Input
                    type="number"
                    min={5}
                    max={120}
                    className="h-8 text-sm"
                    value={config.timeoutSeconds}
                    onChange={(e) =>
                      setConfig((prev) =>
                        prev
                          ? {
                              ...prev,
                              timeoutSeconds: Math.max(
                                5,
                                Math.min(120, Number(e.target.value) || 30),
                              ),
                            }
                          : prev,
                      )
                    }
                  />
                  <p className="text-[10px] text-muted-foreground/60">
                    {t("settings.webSearchTimeoutDesc")}
                  </p>
                </div>

                {/* Cache TTL */}
                <div className="space-y-1">
                  <label className="text-xs font-medium text-muted-foreground">
                    {t("settings.webSearchCacheTtl")}
                  </label>
                  <Input
                    type="number"
                    min={0}
                    max={60}
                    className="h-8 text-sm"
                    value={config.cacheTtlMinutes}
                    onChange={(e) =>
                      setConfig((prev) =>
                        prev
                          ? {
                              ...prev,
                              cacheTtlMinutes: Math.max(
                                0,
                                Math.min(60, Number(e.target.value) || 0),
                              ),
                            }
                          : prev,
                      )
                    }
                  />
                  <p className="text-[10px] text-muted-foreground/60">
                    {t("settings.webSearchCacheTtlDesc")}
                  </p>
                </div>
              </div>

              <div className="grid grid-cols-3 gap-3">
                {/* Default country */}
                <div className="space-y-1">
                  <label className="text-xs font-medium text-muted-foreground">
                    {t("settings.webSearchDefaultCountry")}
                  </label>
                  <Select
                    value={config.defaultCountry ?? "auto"}
                    onValueChange={(v) =>
                      setConfig((prev) =>
                        prev ? { ...prev, defaultCountry: v === "auto" ? null : v } : prev,
                      )
                    }
                  >
                    <SelectTrigger className="h-8 text-sm">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="auto">{t("settings.webSearchCountryAuto")}</SelectItem>
                      <SelectItem value="CN">🇨🇳 China</SelectItem>
                      <SelectItem value="US">🇺🇸 United States</SelectItem>
                      <SelectItem value="JP">🇯🇵 Japan</SelectItem>
                      <SelectItem value="KR">🇰🇷 South Korea</SelectItem>
                      <SelectItem value="GB">🇬🇧 United Kingdom</SelectItem>
                      <SelectItem value="DE">🇩🇪 Germany</SelectItem>
                      <SelectItem value="FR">🇫🇷 France</SelectItem>
                      <SelectItem value="RU">🇷🇺 Russia</SelectItem>
                      <SelectItem value="BR">🇧🇷 Brazil</SelectItem>
                      <SelectItem value="IN">🇮🇳 India</SelectItem>
                      <SelectItem value="AU">🇦🇺 Australia</SelectItem>
                      <SelectItem value="CA">🇨🇦 Canada</SelectItem>
                      <SelectItem value="SG">🇸🇬 Singapore</SelectItem>
                      <SelectItem value="TW">🇹🇼 Taiwan</SelectItem>
                      <SelectItem value="HK">🇭🇰 Hong Kong</SelectItem>
                    </SelectContent>
                  </Select>
                </div>

                {/* Default language */}
                <div className="space-y-1">
                  <label className="text-xs font-medium text-muted-foreground">
                    {t("settings.webSearchDefaultLanguage")}
                  </label>
                  <Select
                    value={config.defaultLanguage ?? "auto"}
                    onValueChange={(v) =>
                      setConfig((prev) =>
                        prev ? { ...prev, defaultLanguage: v === "auto" ? null : v } : prev,
                      )
                    }
                  >
                    <SelectTrigger className="h-8 text-sm">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="auto">
                        {t("settings.webSearchLanguageAuto")} (
                        {SUPPORTED_LANGUAGES.find((l) => l.code === i18n.language)?.label ??
                          i18n.language}
                        )
                      </SelectItem>
                      {SUPPORTED_LANGUAGES.map((lang) => (
                        <SelectItem key={lang.code} value={lang.code.split("-")[0]}>
                          {lang.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>

                {/* Default freshness */}
                <div className="space-y-1">
                  <label className="text-xs font-medium text-muted-foreground">
                    {t("settings.webSearchDefaultFreshness")}
                  </label>
                  <Select
                    value={config.defaultFreshness ?? "none"}
                    onValueChange={(v) =>
                      setConfig((prev) =>
                        prev ? { ...prev, defaultFreshness: v === "none" ? null : v } : prev,
                      )
                    }
                  >
                    <SelectTrigger className="h-8 text-sm">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="none">{t("settings.webSearchFreshnessNone")}</SelectItem>
                      <SelectItem value="day">{t("settings.webSearchFreshnessDay")}</SelectItem>
                      <SelectItem value="week">{t("settings.webSearchFreshnessWeek")}</SelectItem>
                      <SelectItem value="month">{t("settings.webSearchFreshnessMonth")}</SelectItem>
                      <SelectItem value="year">{t("settings.webSearchFreshnessYear")}</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
              </div>
            </div>
          )}
        </div>

        {/* Save button */}
        <div className="flex items-center gap-3 pt-2">
          <Button onClick={handleSave} disabled={!isDirty || saving} size="sm">
            {saving ? (
              <Loader2 className="h-4 w-4 animate-spin mr-1.5" />
            ) : justSaved ? (
              <Check className="h-4 w-4 mr-1.5" />
            ) : null}
            {justSaved ? t("settings.webSearchSaved") : t("common.save")}
          </Button>
        </div>
      </div>
    </div>
  )
}
