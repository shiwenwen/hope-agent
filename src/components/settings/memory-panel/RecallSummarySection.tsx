import { useCallback, useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Check, ChevronRight, Loader2, Save, Sparkles } from "lucide-react"
import { Switch } from "@/components/ui/switch"
import { DeferredNumberInput } from "@/components/ui/deferred-number-input"
import { ModelChainEditor, type ModelChainRef } from "@/components/ui/model-chain-editor"
import type { AvailableModel } from "@/components/ui/model-selector"
import { cn } from "@/lib/utils"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"

interface RecallSummaryConfig {
  enabled: boolean
  minHits: number
  contextCharBudget: number
  timeoutSecs: number
  maxTokens: number
  includeHistory: boolean
  modelOverride?: ModelChainRef | null
}

const DEFAULTS: RecallSummaryConfig = {
  enabled: false,
  minHits: 3,
  contextCharBudget: 20000,
  timeoutSecs: 30,
  maxTokens: 1024,
  includeHistory: true,
  modelOverride: null,
}

function configsEqual(a: RecallSummaryConfig, b: RecallSummaryConfig): boolean {
  return (
    a.enabled === b.enabled &&
    a.minHits === b.minHits &&
    a.contextCharBudget === b.contextCharBudget &&
    a.timeoutSecs === b.timeoutSecs &&
    a.maxTokens === b.maxTokens &&
    a.includeHistory === b.includeHistory &&
    JSON.stringify(a.modelOverride ?? null) === JSON.stringify(b.modelOverride ?? null)
  )
}

/**
 * Opt-in LLM summarization layer on top of `recall_memory`/`session_search`
 * tool output. Was fully implemented backend-side already (min_hits/timeout/
 * budget config, usage-counter in the Dashboard Learning tab) but had no
 * settings-panel home at all — this is its first GUI, including the
 * `enabled` master switch itself.
 */
export default function RecallSummarySection() {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const [config, setConfig] = useState<RecallSummaryConfig>(DEFAULTS)
  const [original, setOriginal] = useState<RecallSummaryConfig>(DEFAULTS)
  const [loaded, setLoaded] = useState(false)
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  const [availableModels, setAvailableModels] = useState<AvailableModel[]>([])

  const load = useCallback(async () => {
    try {
      const cfg = await getTransport().call<RecallSummaryConfig>("get_recall_summary_config")
      setConfig(cfg)
      setOriginal(cfg)
      setLoaded(true)
    } catch (e) {
      logger.error("settings", "RecallSummarySection::load", "Failed to load", e)
      setLoaded(true)
    }
  }, [])

  useEffect(() => {
    load()
  }, [load])

  useEffect(() => {
    getTransport()
      .call<AvailableModel[]>("get_available_models")
      .then(setAvailableModels)
      .catch((e) => logger.error("settings", "RecallSummarySection::loadModels", "Failed to load", e))
  }, [])

  const dirty = useMemo(() => loaded && !configsEqual(config, original), [loaded, config, original])

  const handleSave = async () => {
    setSaving(true)
    try {
      const saved = await getTransport().call<RecallSummaryConfig>(
        "save_recall_summary_config",
        { config },
      )
      setConfig(saved)
      setOriginal(saved)
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "RecallSummarySection::save", "Failed to save", e)
      setSaveStatus("failed")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } finally {
      setSaving(false)
    }
  }

  if (!loaded) return null

  return (
    <div className="mt-6 mb-4 pt-4 border-t border-border/50">
      <Button
        variant="ghost"
        size="sm"
        onClick={() => setExpanded(!expanded)}
        className="h-auto -ml-2 gap-1 px-2 py-1 text-sm font-medium text-muted-foreground hover:bg-transparent hover:text-foreground"
      >
        <ChevronRight className={cn("h-3.5 w-3.5 transition-transform", expanded && "rotate-90")} />
        <Sparkles className="h-3.5 w-3.5 mr-0.5" />
        {t("settings.recallSummary.title", "Recall summarization")}
        {config.enabled && (
          <span className="rounded-full bg-primary/10 px-1.5 text-[10px] text-primary">
            {t("common.on", "On")}
          </span>
        )}
      </Button>

      {expanded && (
        <div className="mt-3 space-y-4">
          <p className="text-xs text-muted-foreground">
            {t(
              "settings.recallSummary.desc",
              "When recall_memory / session_search return many hits, compress them into one concise paragraph via an extra LLM call instead of returning raw snippets. Off by default — costs one call per qualifying search.",
            )}
          </p>

          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="text-xs font-medium">
                {t("settings.recallSummary.enabled", "Enable summarization")}
              </div>
            </div>
            <Switch
              checked={config.enabled}
              onCheckedChange={(v) => setConfig((c) => ({ ...c, enabled: v }))}
              aria-label={t("settings.recallSummary.enabled", "Enable summarization")}
            />
          </div>

          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="text-xs font-medium">
                {t("settings.recallSummary.minHits", "Minimum hits to trigger")}
              </div>
            </div>
            <DeferredNumberInput
              min={1}
              value={config.minHits}
              onValueCommit={(value) => setConfig((c) => ({ ...c, minHits: value }))}
              className="h-7 w-16 text-xs"
            />
          </div>

          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="text-xs font-medium">
                {t("settings.recallSummary.contextCharBudget", "Context budget (chars)")}
              </div>
            </div>
            <DeferredNumberInput
              min={1000}
              max={200000}
              value={config.contextCharBudget}
              onValueCommit={(value) => setConfig((c) => ({ ...c, contextCharBudget: value }))}
              className="h-7 w-24 text-xs"
            />
          </div>

          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="text-xs font-medium">
                {t("settings.recallSummary.timeoutSecs", "Timeout (s)")}
              </div>
            </div>
            <DeferredNumberInput
              min={5}
              max={120}
              value={config.timeoutSecs}
              onValueCommit={(value) => setConfig((c) => ({ ...c, timeoutSecs: value }))}
              className="h-7 w-16 text-xs"
            />
          </div>

          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="text-xs font-medium">
                {t("settings.recallSummary.maxTokens", "Max output tokens")}
              </div>
            </div>
            <DeferredNumberInput
              min={128}
              max={4096}
              value={config.maxTokens}
              onValueCommit={(value) => setConfig((c) => ({ ...c, maxTokens: value }))}
              className="h-7 w-20 text-xs"
            />
          </div>

          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="text-xs font-medium">
                {t("settings.recallSummary.includeHistory", "Also summarize conversation history hits")}
              </div>
            </div>
            <Switch
              checked={config.includeHistory}
              onCheckedChange={(v) => setConfig((c) => ({ ...c, includeHistory: v }))}
              aria-label={t(
                "settings.recallSummary.includeHistory",
                "Also summarize conversation history hits",
              )}
            />
          </div>

          <div className="space-y-1">
            <div className="text-xs font-medium">{t("settings.recallSummary.model", "Model")}</div>
            <ModelChainEditor
              value={config.modelOverride ?? null}
              onChange={(next: ModelChainRef | null) => setConfig((c) => ({ ...c, modelOverride: next }))}
              availableModels={availableModels}
              inheritLabel={t("settings.recallSummary.modelDefault", "Follow automation default")}
            />
          </div>

          <div className="flex items-center justify-end gap-2 pt-2">
            <Button
              onClick={() => void handleSave()}
              disabled={saving || !dirty}
              variant={
                saveStatus === "saved" ? "outline" : saveStatus === "failed" ? "destructive" : "default"
              }
              size="sm"
              className="gap-1.5"
            >
              {saving ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : saveStatus === "saved" ? (
                <Check className="h-3.5 w-3.5 text-green-600" />
              ) : (
                <Save className="h-3.5 w-3.5" />
              )}
              {saveStatus === "saved"
                ? t("common.saved")
                : saveStatus === "failed"
                  ? t("common.retry")
                  : t("common.save")}
            </Button>
          </div>
        </div>
      )}
    </div>
  )
}
