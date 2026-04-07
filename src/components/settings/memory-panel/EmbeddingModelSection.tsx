import { useTranslation } from "react-i18next"
import { getTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Download, Loader2, Check, Zap, Wifi, Sparkles } from "lucide-react"
import TestResultDisplay, { parseTestResult } from "../TestResultDisplay"
import type { useMemoryData } from "./useMemoryData"

type MemoryData = ReturnType<typeof useMemoryData>

interface EmbeddingModelSectionProps {
  data: MemoryData
}

export default function EmbeddingModelSection({ data }: EmbeddingModelSectionProps) {
  const { t } = useTranslation()

  const {
    totalCount,
    embeddingConfig, setEmbeddingConfig,
    presets,
    localModels,
    embeddingDirty, setEmbeddingDirty,
    embeddingTestLoading, setEmbeddingTestLoading,
    embeddingTestResult, setEmbeddingTestResult,
    embeddingSaving,
    embeddingSaveStatus,
    batchLoading,
    saveEmbeddingConfig,
    handleReembedAll,
  } = data

  return (
    <>
      {/* Provider type selector */}
      <div>
        <label className="text-sm font-medium mb-1.5 block">
          {t("settings.memoryEmbeddingProvider")}
        </label>
        <div className="flex flex-wrap gap-2">
          <button
            onClick={() => {
              setEmbeddingConfig({
                ...embeddingConfig,
                providerType: "auto",
                apiBaseUrl: null,
                apiKey: null,
                apiModel: null,
                apiDimensions: null,
              })
              setEmbeddingDirty(true)
            }}
            className={cn(
              "px-3 py-1.5 rounded-lg text-xs border transition-colors flex items-center gap-1",
              embeddingConfig.providerType === "auto"
                ? "border-primary bg-primary/10 text-primary"
                : "border-border text-muted-foreground hover:border-foreground/30",
            )}
          >
            <Sparkles className="h-3 w-3" />
            Auto
          </button>
          {presets.map((preset) => (
            <button
              key={preset.name}
              onClick={() => {
                setEmbeddingConfig({
                  ...embeddingConfig,
                  providerType: preset.providerType,
                  apiBaseUrl: preset.baseUrl,
                  apiKey: null,
                  apiModel: preset.defaultModel,
                  apiDimensions: preset.defaultDimensions,
                })
                setEmbeddingDirty(true)
              }}
              className={cn(
                "px-3 py-1.5 rounded-lg text-xs border transition-colors",
                embeddingConfig.apiBaseUrl === preset.baseUrl
                  ? "border-primary bg-primary/10 text-primary"
                  : "border-border text-muted-foreground hover:border-foreground/30",
              )}
            >
              {preset.name}
            </button>
          ))}
          <button
            onClick={() => {
              setEmbeddingConfig({
                ...embeddingConfig,
                providerType: "local",
                apiBaseUrl: null,
                apiKey: null,
                apiModel: null,
              })
              setEmbeddingDirty(true)
            }}
            className={cn(
              "px-3 py-1.5 rounded-lg text-xs border transition-colors",
              embeddingConfig.providerType === "local"
                ? "border-primary bg-primary/10 text-primary"
                : "border-border text-muted-foreground hover:border-foreground/30",
            )}
          >
            {t("settings.memoryLocalModel")}
          </button>
        </div>
      </div>

      {embeddingConfig.providerType === "auto" ? (
        /* Auto mode info */
        <div className="rounded-lg bg-primary/5 border border-primary/20 p-3">
          <p className="text-xs text-muted-foreground">
            {t("settings.memoryEmbeddingAutoDesc")}
          </p>
        </div>
      ) : embeddingConfig.providerType !== "local" ? (
        /* API mode fields */
        <div className="space-y-3">
          <div>
            <label className="text-xs text-muted-foreground mb-1 block">API Base URL</label>
            <Input
              value={embeddingConfig.apiBaseUrl ?? ""}
              onChange={(e) => {
                setEmbeddingConfig({ ...embeddingConfig, apiBaseUrl: e.target.value })
                setEmbeddingDirty(true)
              }}
              placeholder="https://api.openai.com"
              className="text-sm"
              autoCapitalize="off"
              autoCorrect="off"
              spellCheck={false}
            />
          </div>
          <div>
            <label className="text-xs text-muted-foreground mb-1 block">API Key</label>
            <Input
              type="password"
              value={embeddingConfig.apiKey ?? ""}
              onChange={(e) => {
                setEmbeddingConfig({ ...embeddingConfig, apiKey: e.target.value })
                setEmbeddingDirty(true)
              }}
              placeholder="sk-..."
              className="text-sm"
              autoCapitalize="off"
              autoCorrect="off"
              spellCheck={false}
            />
          </div>
          <div className="flex gap-3">
            <div className="flex-1">
              <label className="text-xs text-muted-foreground mb-1 block">
                {t("settings.memoryModel")}
              </label>
              <Input
                value={embeddingConfig.apiModel ?? ""}
                onChange={(e) => {
                  setEmbeddingConfig({ ...embeddingConfig, apiModel: e.target.value })
                  setEmbeddingDirty(true)
                }}
                placeholder="text-embedding-3-small"
                className="text-sm"
                autoCapitalize="off"
                autoCorrect="off"
                spellCheck={false}
              />
            </div>
            <div className="w-28">
              <label className="text-xs text-muted-foreground mb-1 block">
                {t("settings.memoryDimensions")}
              </label>
              <Input
                type="number"
                value={embeddingConfig.apiDimensions ?? ""}
                onChange={(e) => {
                  setEmbeddingConfig({
                    ...embeddingConfig,
                    apiDimensions: e.target.value ? Number(e.target.value) : null,
                  })
                  setEmbeddingDirty(true)
                }}
                placeholder="1536"
                className="text-sm"
              />
            </div>
          </div>
        </div>
      ) : (
        /* Local model selector */
        <div className="space-y-2">
          <label className="text-sm font-medium mb-1.5 block">
            {t("settings.memorySelectModel")}
          </label>
          {localModels.map((model) => (
            <button
              key={model.id}
              onClick={() => {
                setEmbeddingConfig({
                  ...embeddingConfig,
                  localModelId: model.id,
                  apiDimensions: model.dimensions,
                })
                setEmbeddingDirty(true)
              }}
              className={cn(
                "w-full flex items-center justify-between px-3 py-2.5 rounded-lg border transition-colors text-left",
                embeddingConfig.localModelId === model.id
                  ? "border-primary bg-primary/10"
                  : "border-border hover:border-foreground/30",
              )}
            >
              <div>
                <div className="text-sm font-medium">{model.name}</div>
                <div className="text-xs text-muted-foreground">
                  {model.dimensions}d | {model.sizeMb}MB | RAM {model.minRamGb}GB+ |{" "}
                  {model.languages.join(", ")}
                </div>
              </div>
              {model.downloaded ? (
                <span className="text-xs text-green-500">✓</span>
              ) : (
                <Download className="h-4 w-4 text-muted-foreground" />
              )}
            </button>
          ))}
        </div>
      )}

      {/* Test & Save buttons */}
      <div className="flex items-center gap-2 mt-4">
        <Button
          onClick={saveEmbeddingConfig}
          size="sm"
          disabled={(!embeddingDirty && embeddingSaveStatus === "idle") || embeddingSaving}
          className={cn(
            embeddingSaveStatus === "saved" && "bg-green-500/10 text-green-600 hover:bg-green-500/20",
            embeddingSaveStatus === "failed" && "bg-destructive/10 text-destructive hover:bg-destructive/20",
          )}
        >
          {embeddingSaving ? (
            <span className="flex items-center gap-1.5">
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              {t("common.saving")}
            </span>
          ) : embeddingSaveStatus === "saved" ? (
            <span className="flex items-center gap-1.5">
              <Check className="h-3.5 w-3.5" />
              {t("common.saved")}
            </span>
          ) : embeddingSaveStatus === "failed" ? (
            t("common.saveFailed")
          ) : (
            t("common.save")
          )}
        </Button>
        <Button
          variant="secondary"
          size="sm"
          disabled={
            embeddingTestLoading ||
            (embeddingConfig.providerType === "local"
              ? !embeddingConfig.localModelId
              : !embeddingConfig.apiBaseUrl?.trim())
          }
          onClick={async () => {
            setEmbeddingTestLoading(true)
            setEmbeddingTestResult(null)
            try {
              const msg = await getTransport().call<string>("test_embedding", {
                config: embeddingConfig,
              })
              setEmbeddingTestResult(parseTestResult(msg, false))
            } catch (e) {
              setEmbeddingTestResult(parseTestResult(String(e), true))
            } finally {
              setEmbeddingTestLoading(false)
            }
          }}
        >
          {embeddingTestLoading ? (
            <span className="flex items-center gap-2">
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              {t("common.testing")}
            </span>
          ) : (
            <span className="flex items-center gap-2">
              <Wifi className="h-3.5 w-3.5" />
              {t("settings.memoryEmbeddingTest")}
            </span>
          )}
        </Button>
      </div>

      {embeddingTestResult && (
        <div className="mt-3">
          <TestResultDisplay result={embeddingTestResult} />
        </div>
      )}

      {/* Re-embed All */}
      {embeddingConfig.enabled && totalCount > 0 && (
        <div className="mt-6 pt-4 border-t border-border/50">
          <div className="flex items-center justify-between">
            <div>
              <div className="text-sm font-medium">{t("settings.memoryReembedAll")}</div>
              <div className="text-xs text-muted-foreground">
                {t("settings.memoryCount", { count: totalCount })}
              </div>
            </div>
            <Button
              variant="outline"
              size="sm"
              disabled={batchLoading}
              onClick={handleReembedAll}
            >
              {batchLoading ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin mr-1.5" />
              ) : (
                <Zap className="h-3.5 w-3.5 mr-1.5" />
              )}
              {t("settings.memoryReembedAll")}
            </Button>
          </div>
        </div>
      )}
    </>
  )
}
