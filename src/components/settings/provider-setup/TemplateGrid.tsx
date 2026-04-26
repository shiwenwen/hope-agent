import { useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import ProviderIcon from "@/components/common/ProviderIcon"
import LocalLlmAssistantCard from "@/components/settings/local-llm/LocalLlmAssistantCard"
import { hasLocalOllamaProvider } from "@/components/settings/local-llm/provider-detection"
import { ArrowLeft, CheckCircle2, Globe, Loader2, Search, Settings2 } from "lucide-react"
import { PROVIDER_TEMPLATES } from "./templates"
import { RemoteConnectDialog } from "./RemoteConnectDialog"
import { getConfiguredTemplateKeys, hasConfiguredCodexProvider } from "./configured-provider"
import type { ProviderConfig, ProviderTemplate } from "./types"

function ConfiguredBadge({ label }: { label: string }) {
  return (
    <span className="inline-flex shrink-0 items-center gap-1 rounded-full border border-emerald-500/25 bg-emerald-500/10 px-1.5 py-0.5 text-[10px] font-medium text-emerald-700 dark:text-emerald-300">
      <CheckCircle2 className="h-3 w-3" />
      {label}
    </span>
  )
}

interface TemplateGridProps {
  onSelectTemplate: (template: ProviderTemplate) => void
  onStartCustom: () => void
  onCodexAuth: () => Promise<void>
  configuredProviders: ProviderConfig[]
  onRemoteConnected?: () => void
  onCancel?: () => void
  /** Hide the "Connect to remote server" shortcut (onboarding moves it to its own step). */
  hideRemoteConnect?: boolean
  /** Render the one-click local model assistant as a first-class provider option. */
  showLocalLlmAssistant?: boolean
  onLocalLlmInstalled?: () => void
}

export function TemplateGrid({
  onSelectTemplate,
  onStartCustom,
  onCodexAuth,
  configuredProviders,
  onRemoteConnected,
  onCancel,
  hideRemoteConnect = false,
  showLocalLlmAssistant = false,
  onLocalLlmInstalled,
}: TemplateGridProps) {
  const { t } = useTranslation()
  const [searchQuery, setSearchQuery] = useState("")
  const [codexLoading, setCodexLoading] = useState(false)
  const [codexError, setCodexError] = useState("")
  const [remoteOpen, setRemoteOpen] = useState(false)
  const configuredLabel = t("onboarding.summary.providerDone")

  const configuredTemplateKeys = useMemo(
    () => getConfiguredTemplateKeys(PROVIDER_TEMPLATES, configuredProviders),
    [configuredProviders],
  )
  const codexConfigured = useMemo(
    () => hasConfiguredCodexProvider(configuredProviders),
    [configuredProviders],
  )
  const normalizedSearchQuery = searchQuery.trim().toLowerCase()
  const localLlmSearchText = [
    t("settings.localLlm.title"),
    t("settings.localLlm.subtitle"),
    "ollama qwen gemma local 本地 小模型",
  ]
    .join(" ")
    .toLowerCase()
  const shouldShowLocalLlmAssistant =
    showLocalLlmAssistant &&
    !hasLocalOllamaProvider(configuredProviders) &&
    (!normalizedSearchQuery || localLlmSearchText.includes(normalizedSearchQuery))

  async function handleCodexAuth() {
    setCodexLoading(true)
    setCodexError("")
    try {
      await onCodexAuth()
    } catch (e) {
      setCodexError(String(e))
      setCodexLoading(false)
    }
  }

  const filteredTemplates = normalizedSearchQuery
    ? PROVIDER_TEMPLATES.filter((tmpl) => {
        const name = t(`provider_templates.${tmpl.key}.name`, { defaultValue: tmpl.name })
        const desc = t(`provider_templates.${tmpl.key}.description`, {
          defaultValue: tmpl.description,
        })
        return (
          name.toLowerCase().includes(normalizedSearchQuery) ||
          desc.toLowerCase().includes(normalizedSearchQuery)
        )
      })
    : PROVIDER_TEMPLATES

  return (
    <div className="flex flex-col h-full bg-background">
      {/* Header with title */}
      <div
        className="h-[4.5rem] flex items-end pb-2 px-4 border-b border-border shrink-0 relative"
        data-tauri-drag-region
      >
        {onCancel && (
          <Button
            variant="ghost"
            size="sm"
            onClick={onCancel}
            className="gap-1.5 text-muted-foreground hover:text-foreground z-10"
          >
            <ArrowLeft className="h-4 w-4" />
            {t("common.back")}
          </Button>
        )}
        <div className="absolute inset-0 flex items-center justify-center pointer-events-none">
          <h1 className="text-sm font-semibold tracking-tight text-foreground mt-5">Hope Agent</h1>
        </div>
      </div>

      {/* Scrollable content area */}
      <div className="flex-1 overflow-y-auto">
        {/* Subtitle */}
        <p className="text-sm text-muted-foreground text-center pt-5 pb-3 px-4">
          {t("provider.selectProvider")}
        </p>

        {/* Codex Quick Auth */}
        <div className="px-6 pb-4 max-w-xl mx-auto w-full">
          <Button
            onClick={handleCodexAuth}
            disabled={codexLoading}
            className="w-full h-11 text-sm font-medium bg-primary hover:bg-primary/90"
          >
            {codexLoading ? (
              <span className="flex items-center gap-2">
                <Loader2 className="h-4 w-4 animate-spin" />
                {t("provider.waitingBrowserLogin")}
              </span>
            ) : (
              <span className="flex items-center gap-2">
                <span>{t("provider.codexSignIn")}</span>
                {codexConfigured && <ConfiguredBadge label={configuredLabel} />}
              </span>
            )}
          </Button>
          <p className="text-xs text-amber-500 text-center mt-2">
            {t("provider.codexSecurityWarning")}
          </p>
          {codexError && <p className="text-xs text-red-400 text-center mt-2">{codexError}</p>}

          {!hideRemoteConnect && (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setRemoteOpen(true)}
              className="w-full mt-3 h-9 gap-2 text-xs text-muted-foreground hover:text-foreground"
            >
              <Globe className="h-3.5 w-3.5" />
              {t("provider.connectRemoteServer")}
            </Button>
          )}

          <div className="flex items-center gap-3 mt-4">
            <div className="flex-1 h-px bg-border" />
            <span className="text-xs text-muted-foreground">{t("provider.orSelectProvider")}</span>
            <div className="flex-1 h-px bg-border" />
          </div>
        </div>

        {/* Search */}
        <div className="px-6 pb-3 max-w-3xl mx-auto w-full">
          <div className="relative">
            <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground" />
            <Input
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder={t("provider.searchProviders")}
              className="bg-card pl-9 h-9 text-xs"
            />
          </div>
        </div>

        {/* Template Grid */}
        <div className="px-6 pb-6 max-w-3xl mx-auto w-full">
          <div className="grid grid-cols-2 sm:grid-cols-3 gap-2">
            {shouldShowLocalLlmAssistant && (
              <div className="col-span-2 sm:col-span-3">
                <LocalLlmAssistantCard
                  compact
                  onProviderInstalled={onLocalLlmInstalled ?? (() => {})}
                />
              </div>
            )}

            {filteredTemplates.map((template) => (
              <button
                key={template.key}
                onClick={() => onSelectTemplate(template)}
                className="flex items-center gap-2.5 p-3 rounded-xl border border-border bg-card hover:border-primary/40 hover:bg-secondary/50 text-left transition-all duration-200"
              >
                <ProviderIcon providerKey={template.key} size={24} className="shrink-0" color />
                <div className="min-w-0">
                  <div className="flex items-center gap-1.5">
                    <div className="min-w-0 flex-1 text-xs font-medium text-foreground truncate">
                      {t(`provider_templates.${template.key}.name`, {
                        defaultValue: template.name,
                      })}
                    </div>
                    {configuredTemplateKeys.has(template.key) && (
                      <ConfiguredBadge label={configuredLabel} />
                    )}
                  </div>
                  <div className="text-[10px] text-muted-foreground truncate">
                    {t(`provider_templates.${template.key}.description`, {
                      defaultValue: template.description,
                    })}
                  </div>
                </div>
              </button>
            ))}

            {/* Custom Provider */}
            <button
              onClick={onStartCustom}
              className="flex items-center gap-2.5 p-3 rounded-xl border border-dashed border-border bg-card/50 hover:border-primary/40 hover:bg-secondary/50 text-left transition-all duration-200"
            >
              <div className="w-7 h-7 rounded-lg flex items-center justify-center bg-secondary text-muted-foreground shrink-0">
                <Settings2 className="h-4 w-4" />
              </div>
              <div className="min-w-0">
                <div className="text-xs font-medium text-foreground">{t("provider.custom")}</div>
                <div className="text-[10px] text-muted-foreground">
                  {t("provider.customDescription")}
                </div>
              </div>
            </button>
          </div>
        </div>
      </div>

      <RemoteConnectDialog
        open={remoteOpen}
        onOpenChange={setRemoteOpen}
        onConnected={() => onRemoteConnected?.()}
      />
    </div>
  )
}
