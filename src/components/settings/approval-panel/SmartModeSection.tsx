import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Loader2, Save, X } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { RadioPills } from "@/components/ui/radio-pills"
import { ModelSelector, type AvailableModel } from "@/components/ui/model-selector"
import { IconTip } from "@/components/ui/tooltip"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"

type SmartStrategy = "self_confidence" | "judge_model" | "both"
type SmartFallback = "default" | "ask" | "allow"

interface SmartModeConfig {
  strategy: SmartStrategy
  judgeModel?: {
    providerId: string
    model: string
    extraPrompt?: string | null
  } | null
  fallback: SmartFallback
}

const STRATEGIES: ReadonlyArray<SmartStrategy> = ["self_confidence", "judge_model", "both"]
const FALLBACKS: ReadonlyArray<SmartFallback> = ["default", "ask", "allow"]

export default function SmartModeSection() {
  const { t } = useTranslation()
  const [config, setConfig] = useState<SmartModeConfig | null>(null)
  const [loading, setLoading] = useState(true)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  const [saving, setSaving] = useState(false)
  const [availableModels, setAvailableModels] = useState<AvailableModel[]>([])

  useEffect(() => {
    let cancelled = false
    getTransport()
      .call<SmartModeConfig>("get_smart_mode_config")
      .then((c) => {
        if (cancelled) return
        setConfig({
          strategy: c.strategy ?? "self_confidence",
          judgeModel: c.judgeModel ?? null,
          fallback: c.fallback ?? "default",
        })
      })
      .catch((e) => logger.error("settings", "smartMode", "get_smart_mode_config failed", e))
      .finally(() => {
        if (!cancelled) setLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [])

  useEffect(() => {
    getTransport()
      .call<AvailableModel[]>("get_available_models")
      .then(setAvailableModels)
      .catch((e) => logger.error("settings", "smartMode", "get_available_models failed", e))
  }, [])

  const save = async () => {
    if (!config) return
    setSaving(true)
    try {
      await getTransport().call("set_smart_mode_config", { config })
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "smartMode", "set_smart_mode_config failed", e)
      setSaveStatus("failed")
      setTimeout(() => setSaveStatus("idle"), 2000)
      toast.error(t("common.saveFailed"))
    } finally {
      setSaving(false)
    }
  }

  if (loading || !config) {
    return (
      <section className="rounded-lg border border-border/50 bg-card/40 p-4 flex items-center justify-center text-xs text-muted-foreground py-6">
        <Loader2 className="h-3.5 w-3.5 mr-2 animate-spin" />
        {t("common.loading")}
      </section>
    )
  }

  const judge = config.judgeModel ?? { providerId: "", model: "", extraPrompt: "" }
  const updateJudge = (patch: Partial<typeof judge>) =>
    setConfig({ ...config, judgeModel: { ...judge, ...patch } })

  const showJudge = config.strategy !== "self_confidence"

  return (
    <section className="rounded-lg border border-border/50 bg-card/40 p-4">
      <header className="mb-3">
        <h3 className="text-sm font-medium text-foreground">
          {t("settings.approvalPanel.smartTitle")}
        </h3>
        <p className="text-xs text-muted-foreground mt-0.5">
          {t("settings.approvalPanel.smartDesc")}
        </p>
      </header>

      <div className="space-y-3">
        <div>
          <label className="text-xs font-medium text-foreground/80">
            {t("settings.approvalPanel.smartStrategy")}
          </label>
          <div className="mt-1.5">
            <RadioPills
              value={config.strategy}
              onChange={(s) => setConfig({ ...config, strategy: s })}
              variant="strong"
              ariaLabel={t("settings.approvalPanel.smartStrategy")}
              options={STRATEGIES.map((s) => ({
                value: s,
                label: t(`settings.approvalPanel.strategies.${s}`),
              }))}
            />
          </div>
        </div>

        <div>
          <label className="text-xs font-medium text-foreground/80">
            {t("settings.approvalPanel.smartFallback")}
          </label>
          <div className="mt-1.5">
            <RadioPills
              value={config.fallback}
              onChange={(f) => setConfig({ ...config, fallback: f })}
              variant="strong"
              ariaLabel={t("settings.approvalPanel.smartFallback")}
              options={FALLBACKS.map((f) => ({
                value: f,
                label: t(`settings.approvalPanel.fallbacks.${f}`),
              }))}
            />
          </div>
        </div>

        {showJudge && (
          <div className="space-y-2 rounded-md border border-border/30 p-3 bg-background/40">
            <div className="text-[11px] font-medium text-muted-foreground/80 uppercase tracking-wide">
              {t("settings.approvalPanel.judgeModelTitle")}
            </div>
            <div className="flex items-center gap-2">
              <div className="flex-1 min-w-0">
                <ModelSelector
                  value={judge.providerId && judge.model ? `${judge.providerId}::${judge.model}` : ""}
                  onChange={(providerId, modelId) => updateJudge({ providerId, model: modelId })}
                  availableModels={availableModels}
                  placeholder={t("settings.approvalPanel.judgeModelPlaceholder")}
                  className="h-8 text-xs"
                />
              </div>
              {(judge.providerId || judge.model) && (
                <IconTip label={t("settings.modelChainRestoreInherit")}>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="h-8 w-8 shrink-0 text-muted-foreground/50 hover:text-foreground"
                    onClick={() => updateJudge({ providerId: "", model: "" })}
                  >
                    <X className="h-3.5 w-3.5" />
                  </Button>
                </IconTip>
              )}
            </div>
            <Textarea
              value={judge.extraPrompt ?? ""}
              onChange={(e) => updateJudge({ extraPrompt: e.target.value })}
              placeholder={t("settings.approvalPanel.judgeExtraPromptPlaceholder")}
              className="text-xs min-h-[80px]"
            />
          </div>
        )}
      </div>

      <div className="mt-3 flex justify-end">
        <Button
          size="sm"
          disabled={saving}
          onClick={save}
          className={`h-7 text-xs ${
            saveStatus === "saved" ? "bg-emerald-600 hover:bg-emerald-600/90" : ""
          } ${saveStatus === "failed" ? "bg-destructive hover:bg-destructive/90" : ""}`}
        >
          {saving ? (
            <Loader2 className="h-3.5 w-3.5 mr-1 animate-spin" />
          ) : (
            <Save className="h-3 w-3 mr-1" />
          )}
          {saving
            ? t("common.saving")
            : saveStatus === "saved"
              ? t("common.saved")
              : saveStatus === "failed"
                ? t("common.saveFailed")
                : t("common.save")}
        </Button>
      </div>
    </section>
  )
}
