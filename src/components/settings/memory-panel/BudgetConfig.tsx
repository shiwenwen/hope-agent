import { useCallback, useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { AlertCircle, Check, ChevronRight, Loader2, Ruler, Save } from "lucide-react"
import { cn } from "@/lib/utils"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { toast } from "sonner"
import {
  DEFAULT_MEMORY_BUDGET,
  type MemoryBudgetConfig,
  type SqliteSectionBudgets,
} from "../types"
import MemoryBudgetInputs from "./MemoryBudgetInputs"
import {
  memoryBudgetOperationErrorToast,
  type MemoryBudgetOperationErrorToast,
} from "./memoryBudgetOperationFeedback"

function budgetsEqual(a: SqliteSectionBudgets, b: SqliteSectionBudgets): boolean {
  return (
    a.userProfile === b.userProfile &&
    a.aboutUser === b.aboutUser &&
    a.preferences === b.preferences &&
    a.projectContext === b.projectContext &&
    a.references === b.references
  )
}

function configsEqual(a: MemoryBudgetConfig, b: MemoryBudgetConfig): boolean {
  return (
    a.totalChars === b.totalChars &&
    a.coreMemoryFileChars === b.coreMemoryFileChars &&
    a.sqliteEntryMaxChars === b.sqliteEntryMaxChars &&
    budgetsEqual(a.sqliteSections, b.sqliteSections)
  )
}

export default function BudgetConfig() {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const [config, setConfig] = useState<MemoryBudgetConfig>(DEFAULT_MEMORY_BUDGET)
  const [original, setOriginal] = useState<MemoryBudgetConfig>(DEFAULT_MEMORY_BUDGET)
  const [loaded, setLoaded] = useState(false)
  const [loading, setLoading] = useState(false)
  const [loadError, setLoadError] = useState<MemoryBudgetOperationErrorToast | null>(null)
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")

  const load = useCallback(async () => {
    setLoading(true)
    try {
      const cfg = await getTransport().call<MemoryBudgetConfig>("get_memory_budget_config")
      setConfig(cfg)
      setOriginal(cfg)
      setLoaded(true)
      setLoadError(null)
    } catch (e) {
      logger.error("settings", "BudgetConfig::load", "Failed to load memory budget", e)
      setLoaded(false)
      setLoadError(memoryBudgetOperationErrorToast("load", t, e))
    } finally {
      setLoading(false)
    }
  }, [t])

  useEffect(() => {
    load()
  }, [load])

  const dirty = useMemo(
    () => loaded && !configsEqual(config, original),
    [loaded, config, original],
  )

  const handleSave = async () => {
    setSaving(true)
    try {
      await getTransport().call("save_memory_budget_config", { config })
      setOriginal(config)
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "BudgetConfig::save", "Failed to save memory budget", e)
      setSaveStatus("failed")
      const failureToast = memoryBudgetOperationErrorToast("save", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
      setTimeout(() => setSaveStatus("idle"), 2000)
    } finally {
      setSaving(false)
    }
  }

  const handleReset = () => {
    setConfig(DEFAULT_MEMORY_BUDGET)
  }

  return (
    <div className="mt-6 mb-4 pt-4 border-t border-border/50">
      <Button
        variant="ghost"
        size="sm"
        onClick={() => setExpanded(!expanded)}
        className="h-auto -ml-2 gap-1 px-2 py-1 text-sm font-medium text-muted-foreground hover:bg-transparent hover:text-foreground"
      >
        <ChevronRight className={cn("h-3.5 w-3.5 transition-transform", expanded && "rotate-90")} />
        <Ruler className="h-3.5 w-3.5 mr-0.5" />
        {t("settings.memoryBudget.title")}
      </Button>

      {expanded && (
        <div className="mt-3 space-y-4">
          <p className="text-xs text-muted-foreground">
            {t("settings.memoryBudget.desc")}
          </p>

          {loading ? (
            <div className="flex items-center gap-2 rounded-md border border-border-soft/60 px-3 py-2 text-xs text-muted-foreground">
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              {t("common.loading", "Loading...")}
            </div>
          ) : loadError ? (
            <div className="rounded-md border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs">
              <div className="flex items-center gap-1.5 font-medium text-foreground">
                <AlertCircle className="h-3.5 w-3.5 text-amber-500" />
                {loadError.title}
              </div>
              {loadError.description && (
                <div className="mt-1 break-all text-muted-foreground">{loadError.description}</div>
              )}
              <button
                type="button"
                className="mt-2 font-medium text-foreground underline underline-offset-2"
                onClick={() => void load()}
              >
                {t("common.retry", "Retry")}
              </button>
            </div>
          ) : loaded ? (
            <>
              <MemoryBudgetInputs value={config} onChange={setConfig} />

              <div className="flex items-center justify-end gap-2 pt-2">
                <Button
                  onClick={handleSave}
                  disabled={saving || !dirty}
                  variant={
                    saveStatus === "saved"
                      ? "outline"
                      : saveStatus === "failed"
                        ? "destructive"
                        : "default"
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
                <Button variant="ghost" size="sm" onClick={handleReset} disabled={saving}>
                  {t("settings.memoryBudget.resetToDefaults")}
                </Button>
              </div>
            </>
          ) : null}
        </div>
      )}
    </div>
  )
}
