import { useEffect, useState } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import type { TestResult } from "@/components/settings/TestResultDisplay"
import { logger } from "@/lib/logger"
import type { ApiType, ModelConfig, ProviderConfig, ProviderTemplate, ThinkingStyleType } from "./types"
import { TemplateGrid } from "./TemplateGrid"
import { TemplateConfig } from "./TemplateConfig"
import { CustomWizard } from "./CustomWizard"

// Re-export types and components that external files depend on
export type { ModelConfig } from "./types"
export { SortableModelEditor, ModelEditor } from "./ModelEditor"

export default function ProviderSetup({
  onComplete,
  onCodexAuth,
  onCancel,
  hideRemoteConnect = false,
  embedded = false,
}: {
  onComplete: () => void
  onCodexAuth: () => Promise<void>
  onCancel?: () => void
  /** Hide the "Connect to remote server" shortcut (onboarding moves it to its own step). */
  hideRemoteConnect?: boolean
  /**
   * True when rendered inside another wizard (e.g. onboarding) that already
   * owns the window chrome. Suppresses `data-tauri-drag-region` on internal
   * headers so the host can keep the sub-wizard's back button + stepper
   * visible instead of hiding them with a drag-region hide rule.
   */
  embedded?: boolean
}) {
  const [mode, setMode] = useState<"choose" | "template-config" | "custom">("choose")
  const { t } = useTranslation()

  // Template selection
  const [selectedTemplate, setSelectedTemplate] = useState<ProviderTemplate | null>(null)
  const [configuredProviders, setConfiguredProviders] = useState<ProviderConfig[]>([])

  // Config form (for both template & custom)
  const [customStep, setCustomStep] = useState(0) // 0=type, 1=connection, 2=models
  const [apiType, setApiType] = useState<ApiType>("openai-chat")
  const [providerName, setProviderName] = useState("")
  const [baseUrl, setBaseUrl] = useState("")
  const [apiKey, setApiKey] = useState("")
  const [models, setModels] = useState<ModelConfig[]>([])
  const [testResult, setTestResult] = useState<TestResult | null>(null)
  const [testLoading, setTestLoading] = useState(false)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState("")
  const [modelsExpanded, setModelsExpanded] = useState(false)
  const [thinkingStyle, setThinkingStyle] = useState<ThinkingStyleType>("openai")
  const [showApiKey, setShowApiKey] = useState(false)

  useEffect(() => {
    let cancelled = false

    async function loadConfiguredProviders() {
      try {
        const providers = await getTransport().call<ProviderConfig[]>("get_providers")
        if (!cancelled) {
          setConfiguredProviders(providers)
        }
      } catch (e) {
        logger.warn(
          "provider-setup",
          "ProviderSetup::loadConfiguredProviders",
          "Failed to load configured providers",
          e,
        )
      }
    }

    void loadConfiguredProviders()

    return () => {
      cancelled = true
    }
  }, [])

  // ── Actions ─────────────────────────────────────────────────────

  function selectTemplate(template: ProviderTemplate) {
    setSelectedTemplate(template)
    setProviderName(t(`provider_templates.${template.key}.name`, { defaultValue: template.name }))
    setBaseUrl(template.baseUrl)
    setApiType(template.apiType)
    setModels([...template.models])
    setApiKey("")
    setTestResult(null)
    setError("")
    setModelsExpanded(false)
    setThinkingStyle(template.thinkingStyle || "openai")
    setMode("template-config")
  }

  function startCustom() {
    setSelectedTemplate(null)
    setProviderName("")
    setBaseUrl("https://api.example.com")
    setApiType("openai-chat")
    setModels([])
    setApiKey("")
    setTestResult(null)
    setError("")
    setCustomStep(0)
    setThinkingStyle("openai")
    setMode("custom")
  }

  async function handleSave() {
    if (models.length === 0) return
    setSaving(true)
    setError("")
    try {
      await getTransport().call("add_provider", {
        config: {
          id: "",
          name: providerName,
          apiType,
          baseUrl,
          apiKey: apiKey || "ollama",
          authProfiles: [],
          userAgent: "claude-code/0.1.0",
          thinkingStyle,
          models,
          enabled: true,
        },
      })
      // Set the first model as active
      const providers = await getTransport().call<ProviderConfig[]>("get_providers")
      const latest = providers[providers.length - 1]
      if (latest && latest.models.length > 0) {
        await getTransport().call("set_active_model", {
          providerId: latest.id,
          modelId: latest.models[0].id,
        })
      }
      onComplete()
    } catch (e) {
      setError(String(e))
    } finally {
      setSaving(false)
    }
  }

  // ── Render ────────────────────────────────────────────────────

  if (mode === "choose") {
    return (
      <TemplateGrid
        onSelectTemplate={selectTemplate}
        onStartCustom={startCustom}
        onCodexAuth={onCodexAuth}
        onRemoteConnected={onComplete}
        onCancel={onCancel}
        hideRemoteConnect={hideRemoteConnect}
        configuredProviders={configuredProviders}
      />
    )
  }

  if (mode === "template-config" && selectedTemplate) {
    return (
      <TemplateConfig
        selectedTemplate={selectedTemplate}
        providerName={providerName}
        setProviderName={setProviderName}
        apiType={apiType}
        setApiType={setApiType}
        baseUrl={baseUrl}
        setBaseUrl={setBaseUrl}
        apiKey={apiKey}
        setApiKey={setApiKey}
        models={models}
        setModels={setModels}
        thinkingStyle={thinkingStyle}
        setThinkingStyle={setThinkingStyle}
        showApiKey={showApiKey}
        setShowApiKey={setShowApiKey}
        modelsExpanded={modelsExpanded}
        setModelsExpanded={setModelsExpanded}
        testResult={testResult}
        setTestResult={setTestResult}
        testLoading={testLoading}
        setTestLoading={setTestLoading}
        saving={saving}
        error={error}
        embedded={embedded}
        onBack={() => setMode("choose")}
        onSave={handleSave}
      />
    )
  }

  // Custom wizard
  return (
    <CustomWizard
      customStep={customStep}
      setCustomStep={setCustomStep}
      apiType={apiType}
      setApiType={setApiType}
      providerName={providerName}
      setProviderName={setProviderName}
      baseUrl={baseUrl}
      setBaseUrl={setBaseUrl}
      apiKey={apiKey}
      setApiKey={setApiKey}
      models={models}
      setModels={setModels}
      thinkingStyle={thinkingStyle}
      setThinkingStyle={setThinkingStyle}
      showApiKey={showApiKey}
      setShowApiKey={setShowApiKey}
      testResult={testResult}
      setTestResult={setTestResult}
      testLoading={testLoading}
      setTestLoading={setTestLoading}
      saving={saving}
      error={error}
      embedded={embedded}
      onBack={() => setMode("choose")}
      onSave={handleSave}
    />
  )
}
