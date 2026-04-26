import { useTranslation } from "react-i18next"
import { getTransport } from "@/lib/transport-provider"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
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
import {
  SortableContext,
  verticalListSortingStrategy,
  arrayMove,
} from "@dnd-kit/sortable"
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
  Settings2,
} from "lucide-react"
import { SortableModelEditor } from "./ModelEditor"
import type { ApiType, ModelConfig, ProviderTemplate, ThinkingStyleType } from "./types"

interface TemplateConfigProps {
  selectedTemplate: ProviderTemplate
  providerName: string
  setProviderName: (v: string) => void
  apiType: ApiType
  setApiType: (v: ApiType) => void
  baseUrl: string
  setBaseUrl: (v: string) => void
  apiKey: string
  setApiKey: (v: string) => void
  models: ModelConfig[]
  setModels: (v: ModelConfig[]) => void
  thinkingStyle: ThinkingStyleType
  setThinkingStyle: (v: ThinkingStyleType) => void
  showApiKey: boolean
  setShowApiKey: (v: boolean) => void
  modelsExpanded: boolean
  setModelsExpanded: (v: boolean) => void
  testResult: TestResult | null
  setTestResult: (v: TestResult | null) => void
  testLoading: boolean
  setTestLoading: (v: boolean) => void
  saving: boolean
  error: string
  /** Suppress `data-tauri-drag-region` on the header when nested in a host wizard. */
  embedded?: boolean
  onBack: () => void
  onSave: () => void
}

export function TemplateConfig({
  selectedTemplate,
  providerName,
  setProviderName,
  apiType,
  setApiType,
  baseUrl,
  setBaseUrl,
  apiKey,
  setApiKey,
  models,
  setModels,
  thinkingStyle,
  setThinkingStyle,
  showApiKey,
  setShowApiKey,
  modelsExpanded,
  setModelsExpanded,
  testResult,
  setTestResult,
  testLoading,
  setTestLoading,
  saving,
  error,
  embedded = false,
  onBack,
  onSave,
}: TemplateConfigProps) {
  const { t } = useTranslation()

  const modelSensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 5 } }),
  )

  const canSave =
    (!selectedTemplate.requiresApiKey || apiKey.trim()) &&
    models.length > 0 &&
    models.every((m) => m.id.trim() && m.name.trim())

  function handleModelDragEnd(event: DragEndEvent) {
    const { active, over } = event
    if (!over || active.id === over.id) return
    const oldIndex = models.findIndex((_, i) => `model-${i}` === active.id)
    const newIndex = models.findIndex((_, i) => `model-${i}` === over.id)
    setModels(arrayMove(models, oldIndex, newIndex))
  }

  async function handleTest() {
    setTestLoading(true)
    setTestResult(null)
    try {
      const msg = await getTransport().call<string>("test_provider", {
        config: {
          id: "",
          name: providerName,
          apiType,
          baseUrl,
          apiKey,
          userAgent: "claude-code/0.1.0",
          thinkingStyle,
          models,
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

  return (
    <div className="flex flex-col h-full bg-background">
      {/* Header */}
      <div
        className="h-[4.5rem] flex items-end justify-center pb-2 px-4 border-b border-border shrink-0"
        data-tauri-drag-region={embedded ? undefined : true}
      >
        <span className="text-sm font-semibold text-foreground flex items-center gap-1.5">
          <ProviderIcon providerKey={selectedTemplate.key} size={18} color />
          {t(`provider_templates.${selectedTemplate.key}.name`, {
            defaultValue: selectedTemplate.name,
          })}
        </span>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto px-6 py-6 max-w-lg mx-auto w-full space-y-4">
        {/* Provider info */}
        <div className="bg-card border border-border rounded-xl p-4 space-y-3">
          <div className="space-y-1.5">
            <label className="text-xs font-medium text-muted-foreground">
              {t("provider.name")}
            </label>
            <Input
              value={providerName}
              onChange={(e) => setProviderName(e.target.value)}
              className="bg-background"
            />
          </div>

          <div className="space-y-1.5">
            <label className="text-xs font-medium text-muted-foreground">
              {t("provider.apiType")}
            </label>
            <Select value={apiType} onValueChange={(v) => setApiType(v as ApiType)}>
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
              {!selectedTemplate.requiresApiKey && (
                <span className="text-[10px] text-muted-foreground/60 font-normal">
                  ({t("provider.optional")})
                </span>
              )}
            </label>
            <div className="relative">
              <Input
                type={showApiKey ? "text" : "password"}
                value={apiKey}
                onChange={(e) => setApiKey(e.target.value)}
                placeholder={
                  selectedTemplate.requiresApiKey
                    ? selectedTemplate.apiKeyPlaceholder
                    : t("provider.leaveEmptyNoAuth")
                }
                className="bg-background font-mono text-xs pr-9"
              />
              <Button
                type="button"
                variant="ghost"
                size="icon"
                onClick={() => setShowApiKey(!showApiKey)}
                className="absolute right-1 top-1/2 -translate-y-1/2 h-7 w-7 text-muted-foreground hover:text-foreground"
              >
                {showApiKey ? (
                  <EyeOff className="h-3.5 w-3.5" />
                ) : (
                  <Eye className="h-3.5 w-3.5" />
                )}
              </Button>
            </div>
          </div>

          <div className="space-y-1.5">
            <label className="text-xs font-medium text-muted-foreground flex items-center gap-1.5">
              <Globe className="h-3 w-3" />
              Base URL
            </label>
            <Input
              value={baseUrl}
              onChange={(e) => setBaseUrl(e.target.value)}
              className="bg-background font-mono text-xs"
            />
          </div>

          <div className="space-y-1.5">
            <label className="text-xs font-medium text-muted-foreground flex items-center gap-1.5">
              <Settings2 className="h-3 w-3" />
              {t("provider.thinkingStyle")}
            </label>
            <Select
              value={thinkingStyle}
              onValueChange={(v) => setThinkingStyle(v as ThinkingStyleType)}
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
            disabled={
              testLoading ||
              (selectedTemplate.requiresApiKey && !apiKey.trim()) ||
              !baseUrl.trim()
            }
            className="w-full"
          >
            {testLoading ? (
              <span className="flex items-center gap-2">
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                {t("common.testing")}
              </span>
            ) : (
              t("provider.testConnection")
            )}
          </Button>

          {testResult && <TestResultDisplay result={testResult} />}
        </div>

        {/* Models (collapsed by default for templates, shows summary) */}
        <div className="bg-card border border-border rounded-xl overflow-hidden">
          <Button
            variant="ghost"
            onClick={() => setModelsExpanded(!modelsExpanded)}
            className="h-auto w-full justify-between rounded-none px-4 py-3 text-left font-normal hover:bg-secondary/30"
          >
            <div className="flex items-center gap-1.5">
              <span className="text-sm font-semibold text-foreground">
                {t("model.modelList")}
              </span>
              <span className="text-[10px] text-muted-foreground/60 bg-secondary/80 px-1.5 py-0.5 rounded-md">
                {models.length}
              </span>
              {models.length > 1 && (
                <span className="text-[10px] text-muted-foreground/50">
                  {t("common.dragToSort")}
                </span>
              )}
            </div>
            <ArrowRight
              className={`h-3.5 w-3.5 text-muted-foreground transition-transform ${
                modelsExpanded ? "rotate-90" : ""
              }`}
            />
          </Button>

          {!modelsExpanded && (
            <div className="px-4 pb-3 flex flex-wrap gap-1.5">
              {models.map((m) => (
                <span
                  key={m.id}
                  className="px-2 py-0.5 text-[10px] rounded-md bg-secondary text-muted-foreground"
                >
                  {m.name}
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
                  items={models.map((_, i) => `model-${i}`)}
                  strategy={verticalListSortingStrategy}
                >
                  {models.map((model, i) => (
                    <SortableModelEditor
                      key={`model-${i}`}
                      sortableId={`model-${i}`}
                      model={model}
                      onChange={(m) => {
                        const updated = [...models]
                        updated[i] = m
                        setModels(updated)
                      }}
                      onRemove={() => setModels(models.filter((_, j) => j !== i))}
                      onTest={
                        baseUrl.trim()
                          ? (modelId) =>
                              getTransport().call<string>("test_model", {
                                config: {
                                  id: "",
                                  name: providerName,
                                  apiType,
                                  baseUrl,
                                  apiKey: apiKey || "ollama",
                                  userAgent: "claude-code/0.1.0",
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
                  setModels([
                    ...models,
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
      <div className="border-t border-border px-6 py-3 flex items-center justify-between gap-2 shrink-0">
        <Button onClick={onBack}>
          <ArrowLeft className="h-4 w-4 mr-1" />
          {t("common.back")}
        </Button>
        <Button onClick={onSave} disabled={!canSave || saving}>
          {saving ? (
            <span className="flex items-center gap-2">
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              {t("common.saving")}
            </span>
          ) : (
            <>
              <Check className="h-4 w-4 mr-1" />
              {t("common.done")}
            </>
          )}
        </Button>
      </div>
    </div>
  )
}
