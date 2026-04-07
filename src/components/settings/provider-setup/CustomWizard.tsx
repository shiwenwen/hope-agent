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
import type { ApiType, ModelConfig, ThinkingStyleType } from "./types"

interface CustomWizardProps {
  customStep: number
  setCustomStep: (v: number) => void
  apiType: ApiType
  setApiType: (v: ApiType) => void
  providerName: string
  setProviderName: (v: string) => void
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
  testResult: TestResult | null
  setTestResult: (v: TestResult | null) => void
  testLoading: boolean
  setTestLoading: (v: boolean) => void
  saving: boolean
  error: string
  onBack: () => void
  onSave: () => void
}

export function CustomWizard({
  customStep,
  setCustomStep,
  apiType,
  setApiType,
  providerName,
  setProviderName,
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
  testResult,
  setTestResult,
  testLoading,
  setTestLoading,
  saving,
  error,
  onBack,
  onSave,
}: CustomWizardProps) {
  const { t } = useTranslation()

  const modelSensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 5 } }),
  )

  const API_TYPE_OPTIONS: {
    value: ApiType
    label: string
    description: string
  }[] = [
    {
      value: "anthropic",
      label: "Anthropic Messages API",
      description: t("wizard.anthropicDesc"),
    },
    {
      value: "openai-chat",
      label: "OpenAI Chat Completions",
      description: t("wizard.openaiChatDesc"),
    },
    {
      value: "openai-responses",
      label: "OpenAI Responses API",
      description: t("wizard.openaiResponsesDesc"),
    },
  ]

  const canNext =
    customStep === 0
      ? true
      : customStep === 1
        ? baseUrl.trim() && providerName.trim()
        : models.length > 0 && models.every((m) => m.id.trim() && m.name.trim())

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

  return (
    <div className="flex flex-col h-full bg-background">
      {/* Header */}
      <div
        className="h-[4.5rem] flex items-end pb-2 px-4 border-b border-border shrink-0"
        data-tauri-drag-region
      >
        <Button
          variant="ghost"
          size="sm"
          onClick={() => {
            if (customStep > 0) setCustomStep(customStep - 1)
            else onBack()
          }}
          className="gap-1.5 text-muted-foreground hover:text-foreground"
        >
          <ArrowLeft className="h-4 w-4" />
          {t("common.back")}
        </Button>

        <div className="flex items-center gap-2 mx-auto">
          {[t("wizard.apiType"), t("wizard.connectionConfig"), t("wizard.models")].map(
            (label, i) => (
              <div key={i} className="flex items-center gap-2">
                <div
                  className={`w-6 h-6 rounded-full flex items-center justify-center text-[10px] font-medium transition-colors ${
                    i === customStep
                      ? "bg-primary text-primary-foreground"
                      : i < customStep
                        ? "bg-primary/20 text-primary"
                        : "bg-secondary text-muted-foreground"
                  }`}
                >
                  {i < customStep ? <Check className="h-3 w-3" /> : i + 1}
                </div>
                <span
                  className={`text-xs hidden sm:inline ${i === customStep ? "text-foreground font-medium" : "text-muted-foreground"}`}
                >
                  {label}
                </span>
                {i < 2 && <div className="w-6 h-px bg-border hidden sm:block" />}
              </div>
            ),
          )}
        </div>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto px-6 py-6 max-w-lg mx-auto w-full">
        {customStep === 0 && (
          <div className="space-y-3">
            <h3 className="text-base font-semibold text-foreground">{t("wizard.selectApiType")}</h3>
            <div className="grid gap-2.5">
              {API_TYPE_OPTIONS.map((opt) => (
                <button
                  key={opt.value}
                  onClick={() => setApiType(opt.value)}
                  className={`flex items-center gap-3 p-3.5 rounded-xl border text-left transition-all duration-200 ${
                    apiType === opt.value
                      ? "border-primary bg-primary/5 ring-1 ring-primary/30"
                      : "border-border bg-card hover:border-primary/40 hover:bg-secondary/50"
                  }`}
                >
                  <div className="min-w-0">
                    <div className="text-sm font-medium text-foreground">{opt.label}</div>
                    <div className="text-xs text-muted-foreground">{opt.description}</div>
                  </div>
                  {apiType === opt.value && (
                    <Check className="h-4 w-4 text-primary ml-auto shrink-0" />
                  )}
                </button>
              ))}
            </div>
          </div>
        )}

        {customStep === 1 && (
          <div className="space-y-4">
            <h3 className="text-base font-semibold text-foreground">
              {t("wizard.connectionConfig")}
            </h3>
            <div className="space-y-3">
              <div className="space-y-1.5">
                <label className="text-xs font-medium text-muted-foreground">
                  {t("provider.name")}
                </label>
                <Input
                  value={providerName}
                  onChange={(e) => setProviderName(e.target.value)}
                  placeholder={t("provider.myCustomProvider")}
                  className="bg-card"
                />
              </div>
              <div className="space-y-1.5">
                <label className="text-xs font-medium text-muted-foreground flex items-center gap-1.5">
                  <Globe className="h-3 w-3" />
                  Base URL
                </label>
                <Input
                  value={baseUrl}
                  onChange={(e) => setBaseUrl(e.target.value)}
                  placeholder="https://api.example.com"
                  className="bg-card font-mono text-xs"
                />
              </div>
              <div className="space-y-1.5">
                <label className="text-xs font-medium text-muted-foreground flex items-center gap-1.5">
                  <Key className="h-3 w-3" />
                  API Key
                  <span className="text-[10px] text-muted-foreground/60 font-normal">
                    ({t("provider.optional")})
                  </span>
                </label>
                <div className="relative">
                  <Input
                    type={showApiKey ? "text" : "password"}
                    value={apiKey}
                    onChange={(e) => setApiKey(e.target.value)}
                    placeholder={t("provider.authRequired")}
                    className="bg-card font-mono text-xs pr-9"
                  />
                  <button
                    type="button"
                    onClick={() => setShowApiKey(!showApiKey)}
                    className="absolute right-2.5 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground transition-colors"
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
                  <Settings2 className="h-3 w-3" />
                  {t("provider.thinkingStyle")}
                </label>
                <Select
                  value={thinkingStyle}
                  onValueChange={(v) => setThinkingStyle(v as ThinkingStyleType)}
                >
                  <SelectTrigger className="bg-card text-xs font-medium">
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
              <Button
                variant="secondary"
                size="sm"
                onClick={handleTest}
                disabled={testLoading || !apiKey.trim() || !baseUrl.trim()}
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
          </div>
        )}

        {customStep === 2 && (
          <div className="space-y-3">
            <div className="flex items-center justify-between">
              <div>
                <h3 className="text-base font-semibold text-foreground">{t("model.addModels")}</h3>
                <p className="text-xs text-muted-foreground mt-0.5">
                  {t("model.configModels")}
                  {models.length > 1 && (
                    <span className="ml-2 text-[10px] text-muted-foreground/50">
                      {t("common.dragToSort")}
                    </span>
                  )}
                </p>
              </div>
              <Button
                variant="secondary"
                size="sm"
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
                {t("common.add")}
              </Button>
            </div>
            <div className="space-y-2.5 max-h-[400px] overflow-y-auto pr-1">
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
              {models.length === 0 && (
                <div className="text-center py-8 text-muted-foreground text-xs">
                  {t("model.atLeastOneModel")}
                </div>
              )}
            </div>
          </div>
        )}

        {error && <p className="text-xs text-red-400 mt-3">{error}</p>}
      </div>

      {/* Footer */}
      <div className="border-t border-border px-6 py-3 flex justify-end gap-2 shrink-0">
        {customStep < 2 ? (
          <Button onClick={() => setCustomStep(customStep + 1)} disabled={!canNext}>
            {t("common.nextStep")}
            <ArrowRight className="h-4 w-4 ml-1" />
          </Button>
        ) : (
          <Button onClick={onSave} disabled={!canNext || saving}>
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
        )}
      </div>
    </div>
  )
}
