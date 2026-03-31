import { useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { IconTip } from "@/components/ui/tooltip"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { SortableModelEditor, type ModelConfig } from "@/components/settings/provider-setup"
import ProviderIcon from "@/components/common/ProviderIcon"
import TestResultDisplay, {
  parseTestResult,
  type TestResult,
} from "@/components/settings/TestResultDisplay"
import {
  DndContext,
  closestCenter,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core"
import { SortableContext, verticalListSortingStrategy, arrayMove } from "@dnd-kit/sortable"
import {
  ArrowLeft,
  ArrowRight,
  Check,
  Eye,
  EyeOff,
  Globe,
  Key,
  Loader2,
  Plus,
  RefreshCw,
  Settings2,
  Wifi,
} from "lucide-react"

// ── Types ─────────────────────────────────────────────────────────

type ApiType = "anthropic" | "openai-chat" | "openai-responses" | "codex"
type ThinkingStyleType = "openai" | "anthropic" | "zai" | "qwen" | "none"

interface ProviderConfig {
  id: string
  name: string
  apiType: ApiType
  baseUrl: string
  apiKey: string
  models: ModelConfig[]
  enabled: boolean
  userAgent: string
  thinkingStyle: ThinkingStyleType
}

// ── Main Component ────────────────────────────────────────────────

export default function ProviderEditPage({
  provider,
  onSave,
  onCancel,
  onCodexReauth,
}: {
  provider: ProviderConfig
  onSave: () => void
  onCancel: () => void
  onCodexReauth?: () => void
}) {
  const { t } = useTranslation()

  // Edit form state — initialized from provider
  const [editName, setEditName] = useState(provider.name)
  const [editBaseUrl, setEditBaseUrl] = useState(provider.baseUrl)
  const [editApiKey, setEditApiKey] = useState(provider.apiKey)
  const [editApiType, setEditApiType] = useState<ApiType>(provider.apiType)
  const [editUserAgent, setEditUserAgent] = useState(provider.userAgent || "claude-code/0.1.0")
  const [editThinkingStyle, setEditThinkingStyle] = useState<ThinkingStyleType>(
    provider.thinkingStyle || "openai",
  )
  const [editModels, setEditModels] = useState<ModelConfig[]>([...provider.models])
  const [saving, setSaving] = useState(false)
  const [testResult, setTestResult] = useState<TestResult | null>(null)
  const [testLoading, setTestLoading] = useState(false)
  const [showApiKey, setShowApiKey] = useState(false)
  const [modelsExpanded, setModelsExpanded] = useState(false)
  const [error, setError] = useState("")

  const modelSensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 5 } }),
  )

  function handleModelDragEnd(event: DragEndEvent) {
    const { active, over } = event
    if (!over || active.id === over.id) return
    const oldIndex = editModels.findIndex((_, i) => `model-${i}` === active.id)
    const newIndex = editModels.findIndex((_, i) => `model-${i}` === over.id)
    setEditModels(arrayMove(editModels, oldIndex, newIndex))
  }

  async function handleTest() {
    setTestLoading(true)
    setTestResult(null)
    try {
      const msg = await invoke<string>("test_provider", {
        config: {
          id: provider.id,
          name: editName,
          apiType: editApiType,
          baseUrl: editBaseUrl,
          apiKey: editApiKey,
          userAgent: editUserAgent,
          thinkingStyle: editThinkingStyle,
          models: [],
          enabled: true,
        },
      })
      setTestResult(parseTestResult(msg, false))
    } catch (e) {
      setTestResult(parseTestResult(String(e), true))
    } finally {
      setTestLoading(false)
    }
  }

  async function handleSave() {
    setSaving(true)
    setError("")
    try {
      await invoke("update_provider", {
        config: {
          ...provider,
          name: editName,
          apiType: editApiType,
          baseUrl: editBaseUrl,
          apiKey: editApiKey,
          userAgent: editUserAgent,
          thinkingStyle: editThinkingStyle,
          models: editModels,
        },
      })
      onSave()
    } catch (e) {
      setError(String(e))
    } finally {
      setSaving(false)
    }
  }

  const isCodex = provider.apiType === "codex"

  return (
    <div className="flex flex-col h-full bg-background">
      {/* Header */}
      <div className="h-11 flex items-center px-4 border-b border-border shrink-0" data-tauri-drag-region>
        <Button
          variant="ghost"
          size="sm"
          onClick={onCancel}
          className="gap-1.5 text-muted-foreground hover:text-foreground"
        >
          <ArrowLeft className="h-4 w-4" />
          {t("common.back")}
        </Button>
        <span className="text-sm font-semibold text-foreground mx-auto flex items-center gap-1.5">
          <ProviderIcon providerName={provider.name} size={18} color />
          {t("provider.editProvider")}
        </span>
        <div className="w-12 flex justify-end">
          {isCodex && onCodexReauth && (
            <IconTip label={t("provider.relogin")}>
                <Button
                  variant="ghost"
                  size="icon"
                  className="h-7 w-7"
                  onClick={() => {
                    onCancel()
                    onCodexReauth()
                  }}
                >
                  <RefreshCw className="h-3.5 w-3.5" />
                </Button>
              </IconTip>
          )}
        </div>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto px-6 py-6 space-y-4">
        {/* Provider info */}
        <div className="bg-card border border-border rounded-xl p-4 space-y-3">
          <div className="space-y-1.5">
            <label className="text-xs font-medium text-muted-foreground">
              {t("provider.name")}
            </label>
            <Input
              value={editName}
              onChange={(e) => setEditName(e.target.value)}
              className="bg-background"
            />
          </div>

          {!isCodex && (
            <>
              <div className="space-y-1.5">
                <label className="text-xs font-medium text-muted-foreground">
                  {t("provider.apiType")}
                </label>
                <Select value={editApiType} onValueChange={(v) => setEditApiType(v as ApiType)}>
                  <SelectTrigger className="bg-background text-xs font-medium">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="openai-chat">OpenAI Chat Completions</SelectItem>
                    <SelectItem value="openai-responses">OpenAI Responses API</SelectItem>
                    <SelectItem value="anthropic">Anthropic Messages API</SelectItem>
                  </SelectContent>
                </Select>
              </div>

              <div className="space-y-1.5">
                <label className="text-xs font-medium text-muted-foreground flex items-center gap-1.5">
                  <Key className="h-3 w-3" />
                  API Key
                </label>
                <div className="relative">
                  <Input
                    type={showApiKey ? "text" : "password"}
                    value={editApiKey}
                    onChange={(e) => setEditApiKey(e.target.value)}
                    className="bg-background font-mono text-xs pr-8"
                  />
                  <button
                    type="button"
                    onClick={() => setShowApiKey(!showApiKey)}
                    className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground transition-colors"
                  >
                    {showApiKey ? (
                      <EyeOff className="h-3.5 w-3.5" />
                    ) : (
                      <Eye className="h-3.5 w-3.5" />
                    )}
                  </button>
                </div>
              </div>

              <div className="space-y-1.5">
                <label className="text-xs font-medium text-muted-foreground flex items-center gap-1.5">
                  <Globe className="h-3 w-3" />
                  Base URL
                </label>
                <Input
                  value={editBaseUrl}
                  onChange={(e) => setEditBaseUrl(e.target.value)}
                  className="bg-background font-mono text-xs"
                />
              </div>

              <div className="space-y-1.5">
                <label className="text-xs font-medium text-muted-foreground flex items-center gap-1.5">
                  <Settings2 className="h-3 w-3" />
                  User-Agent
                </label>
                <Input
                  value={editUserAgent}
                  onChange={(e) => setEditUserAgent(e.target.value)}
                  placeholder="claude-code/0.1.0"
                  className="bg-background font-mono text-xs"
                />
              </div>

              <div className="space-y-1.5">
                <label className="text-xs font-medium text-muted-foreground flex items-center gap-1.5">
                  <Settings2 className="h-3 w-3" />
                  {t("provider.thinkingStyle")}
                </label>
                <Select
                  value={editThinkingStyle}
                  onValueChange={(v) => setEditThinkingStyle(v as ThinkingStyleType)}
                >
                  <SelectTrigger className="bg-background text-xs font-medium">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="openai">OpenAI (reasoning_effort)</SelectItem>
                    <SelectItem value="anthropic">Anthropic (thinking budget)</SelectItem>
                    <SelectItem value="zai">Z.AI (thinking budget)</SelectItem>
                    <SelectItem value="qwen">Qwen (enable_thinking)</SelectItem>
                    <SelectItem value="none">{t("provider.thinkingStyleNone")}</SelectItem>
                  </SelectContent>
                </Select>
              </div>

              {/* Test Connection */}
              <Button
                variant="secondary"
                size="sm"
                onClick={handleTest}
                disabled={testLoading || !editBaseUrl.trim()}
                className="w-full"
              >
                {testLoading ? (
                  <span className="flex items-center gap-2">
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    {t("common.testing")}
                  </span>
                ) : (
                  <span className="flex items-center gap-2">
                    <Wifi className="h-3.5 w-3.5" />
                    {t("provider.testConnection")}
                  </span>
                )}
              </Button>

              {testResult && <TestResultDisplay result={testResult} />}
            </>
          )}
        </div>

        {/* Models (collapsible like ProviderSetup) */}
        <div className="bg-card border border-border rounded-xl overflow-hidden">
          <button
            onClick={() => setModelsExpanded(!modelsExpanded)}
            className="w-full flex items-center justify-between px-4 py-3 text-left hover:bg-secondary/30 transition-colors"
          >
            <div className="flex items-center gap-1.5">
              <span className="text-sm font-semibold text-foreground">{t("model.modelList")}</span>
              <span className="text-[10px] text-muted-foreground/60 bg-secondary/80 px-1.5 py-0.5 rounded-md">
                {editModels.length}
              </span>
              {editModels.length > 1 && (
                <span className="text-[10px] text-muted-foreground/50">
                  {t("common.dragToSort")}
                </span>
              )}
            </div>
            <ArrowRight
              className={`h-3.5 w-3.5 text-muted-foreground transition-transform ${modelsExpanded ? "rotate-90" : ""}`}
            />
          </button>

          {!modelsExpanded && (
            <div className="px-4 pb-3 flex flex-wrap gap-1.5">
              {editModels.map((m) => (
                <span
                  key={m.id}
                  className="px-2 py-0.5 text-[10px] rounded-md bg-secondary text-muted-foreground"
                >
                  {m.name || m.id}
                </span>
              ))}
            </div>
          )}

          {modelsExpanded && (
            <div className="px-4 pb-4 space-y-2.5">
              <DndContext
                sensors={modelSensors}
                collisionDetection={closestCenter}
                onDragEnd={handleModelDragEnd}
              >
                <SortableContext
                  items={editModels.map((_, i) => `model-${i}`)}
                  strategy={verticalListSortingStrategy}
                >
                  {editModels.map((model, i) => (
                    <SortableModelEditor
                      key={`model-${i}`}
                      sortableId={`model-${i}`}
                      model={model}
                      onChange={(m) => {
                        const updated = [...editModels]
                        updated[i] = m
                        setEditModels(updated)
                      }}
                      onRemove={() => setEditModels(editModels.filter((_, j) => j !== i))}
                      onTest={
                        editBaseUrl.trim() && !isCodex
                          ? (modelId) =>
                              invoke<string>("test_model", {
                                config: {
                                  id: provider.id,
                                  name: editName,
                                  apiType: editApiType,
                                  baseUrl: editBaseUrl,
                                  apiKey: editApiKey,
                                  userAgent: editUserAgent,
                                  thinkingStyle: editThinkingStyle,
                                  models: [],
                                  enabled: true,
                                },
                                modelId,
                              })
                          : undefined
                      }
                    />
                  ))}
                </SortableContext>
              </DndContext>
              <Button
                variant="secondary"
                size="sm"
                className="w-full"
                onClick={() =>
                  setEditModels([
                    ...editModels,
                    {
                      id: "",
                      name: "",
                      inputTypes: ["text"],
                      contextWindow: 128000,
                      maxTokens: 8192,
                      reasoning: false,
                      costInput: 0,
                      costOutput: 0,
                    },
                  ])
                }
              >
                <Plus className="h-3.5 w-3.5 mr-1" />
                {t("model.addModel")}
              </Button>
            </div>
          )}
        </div>

        {error && <p className="text-xs text-red-400">{error}</p>}
      </div>

      {/* Footer */}
      <div className="border-t border-border px-6 py-3 flex justify-end gap-2 shrink-0">
        <Button variant="secondary" onClick={onCancel}>
          {t("common.cancel")}
        </Button>
        <Button onClick={handleSave} disabled={saving}>
          {saving ? (
            <span className="flex items-center gap-2">
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              {t("common.saving")}
            </span>
          ) : (
            <>
              <Check className="h-4 w-4 mr-1" />
              {t("common.save")}
            </>
          )}
        </Button>
      </div>
    </div>
  )
}
