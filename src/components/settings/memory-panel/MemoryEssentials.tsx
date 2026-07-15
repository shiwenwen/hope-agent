import { useCallback, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Brain, FileText, Loader2, RefreshCw, Sparkles } from "lucide-react"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type {
  MemoryLearningMode,
  MemoryRecallMode,
  MemoryRuntimeConfig,
  PendingMemoryCandidatePage,
} from "./types"

interface MemoryEssentialsProps {
  onManage: () => void
}

export default function MemoryEssentials({ onManage }: MemoryEssentialsProps) {
  const { t } = useTranslation()
  const [config, setConfig] = useState<MemoryRuntimeConfig | null>(null)
  const [pendingCount, setPendingCount] = useState(0)
  const [unassignedCount, setUnassignedCount] = useState(0)
  const [lastRecall, setLastRecall] = useState<{
    candidateCount: number
    selectedCount: number
    latencyMs?: number
    mode: MemoryRecallMode | "skip"
  } | null>(null)
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)

  const load = useCallback(async () => {
    setLoading(true)
    try {
      const [runtime, pending] = await Promise.all([
        getTransport().call<MemoryRuntimeConfig>("get_memory_runtime_config"),
        getTransport().call<PendingMemoryCandidatePage>("pending_memory_list_cmd", {
          status: "pending",
          offset: 0,
          limit: 100,
        }),
      ])
      setConfig(runtime)
      setPendingCount(pending.total)
      const reasonCounts = pending.reasonCounts
      setUnassignedCount(reasonCounts
        ? (reasonCounts.project_scope_missing ?? 0) + (reasonCounts.scope_uncertain ?? 0)
        : pending.items.filter((item) =>
            item.reason === "project_scope_missing" || item.reason === "scope_uncertain"
          ).length)
    } catch (error) {
      logger.error("settings", "MemoryEssentials::load", "Failed to load memory essentials", error)
      toast.error(t("settings.memoryV2.loadFailed"))
    } finally {
      setLoading(false)
    }
  }, [t])

  useEffect(() => {
    void load()
    const unlistenPending = getTransport().listen("memory:pending_changed", () => void load())
    const unlistenLearning = getTransport().listen("memory:learning_candidate_created", () =>
      void load(),
    )
    const unlistenRecall = getTransport().listen("memory:recall_completed", (raw) => {
      const event = raw as {
        candidateCount?: number
        selectedCount?: number
        latencyMs?: number
        mode?: string
      }
      setLastRecall({
        candidateCount: event.candidateCount ?? 0,
        selectedCount: event.selectedCount ?? 0,
        latencyMs: event.latencyMs,
        mode: event.mode === "deep" || event.mode === "skip" ? event.mode : "fast",
      })
    })
    return () => {
      unlistenPending()
      unlistenLearning()
      unlistenRecall()
    }
  }, [load])

  const save = async (next: MemoryRuntimeConfig) => {
    const previous = config
    setConfig(next)
    setSaving(true)
    try {
      const saved = await getTransport().call<MemoryRuntimeConfig>("save_memory_runtime_config", {
        config: next,
      })
      setConfig(saved)
    } catch (error) {
      setConfig(previous)
      logger.error("settings", "MemoryEssentials::save", "Failed to save memory runtime", error)
      toast.error(t("settings.memoryV2.saveFailed"))
    } finally {
      setSaving(false)
    }
  }

  const update = (mutate: (draft: MemoryRuntimeConfig) => void) => {
    if (!config || saving) return
    const next = structuredClone(config)
    mutate(next)
    void save(next)
  }

  if (loading || !config) {
    return (
      <div className="flex min-h-28 items-center justify-center rounded-lg border border-border/60 bg-card">
        <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
      </div>
    )
  }

  const learningMode = config.learning.mode
  return (
    <section className="space-y-3" aria-label={t("settings.memoryV2.title")}>
      <div className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-border/60 bg-card px-3 py-3">
        <div>
          <div className="text-sm font-semibold">{t("settings.memoryV2.title")}</div>
          <div className="mt-0.5 text-xs text-muted-foreground">
            {t("settings.memoryV2.subtitle")}
          </div>
        </div>
        <div className="flex items-center gap-2">
          {saving && <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />}
          <span className="text-xs text-muted-foreground">
            {config.enabled ? t("common.on", "On") : t("common.off", "Off")}
          </span>
          <Switch
            checked={config.enabled}
            disabled={saving}
            aria-label={t("settings.memoryV2.title")}
            onCheckedChange={(enabled) => update((draft) => { draft.enabled = enabled })}
          />
        </div>
      </div>

      <div className="grid gap-3 lg:grid-cols-3">
        <div className="rounded-lg border border-border/60 bg-card p-3">
          <div className="flex items-start justify-between gap-3">
            <div className="flex min-w-0 gap-2.5">
              <FileText className="mt-0.5 h-4 w-4 shrink-0 text-primary" />
              <div>
                <div className="text-sm font-medium">{t("settings.memoryV2.core.title")}</div>
                <p className="mt-1 text-xs leading-5 text-muted-foreground">
                  {t("settings.memoryV2.core.desc")}
                </p>
              </div>
            </div>
            <Switch
              checked={config.enabled && config.core.enabled}
              disabled={!config.enabled || saving}
              aria-label={t("settings.memoryV2.core.title")}
              onCheckedChange={(enabled) => update((draft) => { draft.core.enabled = enabled })}
            />
          </div>
          <div className="mt-3 flex items-center justify-between gap-2 text-[11px] text-muted-foreground">
            <span>{t("settings.memoryV2.core.budget", { count: config.core.totalTokens })}</span>
            <Button type="button" size="sm" variant="outline" className="h-7 px-2 text-[11px]" onClick={onManage}>
              {t("settings.memoryV2.core.manage")}
            </Button>
          </div>
        </div>

        <div className="rounded-lg border border-border/60 bg-card p-3">
          <div className="flex gap-2.5">
            <Brain className="mt-0.5 h-4 w-4 shrink-0 text-primary" />
            <div className="min-w-0 flex-1">
              <div className="text-sm font-medium">{t("settings.memoryV2.recall.title")}</div>
              <p className="mt-1 text-xs leading-5 text-muted-foreground">
                {t("settings.memoryV2.recall.desc")}
              </p>
            </div>
          </div>
          <div className="mt-3 space-y-2">
            <div className="flex items-center justify-between gap-2">
              <span className="text-xs">{t("settings.memoryV2.recall.fast")}</span>
              <Switch
                checked={config.enabled && config.recall.enabled}
                disabled={!config.enabled || saving}
                aria-label={t("settings.memoryV2.recall.fast")}
                onCheckedChange={(enabled) => update((draft) => {
                  draft.recall.enabled = enabled
                  draft.recall.userConfigured = true
                })}
              />
            </div>
            <div className="flex items-center justify-between gap-2">
              <span className="text-xs">{t("settings.memoryV2.recall.deep")}</span>
              <Switch
                checked={config.enabled && config.deepRecall.enabled}
                disabled={!config.enabled || !config.recall.enabled || saving}
                aria-label={t("settings.memoryV2.recall.deep")}
                onCheckedChange={(enabled) => update((draft) => { draft.deepRecall.enabled = enabled })}
              />
            </div>
            <p className="text-[11px] leading-4 text-amber-700 dark:text-amber-300">
              {t("settings.memoryV2.recall.deepCost")}
            </p>
            <p className="text-[11px] leading-4 text-muted-foreground">
              {lastRecall
                ? t("settings.memoryV2.recall.last", {
                    selected: lastRecall.selectedCount,
                    candidates: lastRecall.candidateCount,
                    latency: lastRecall.latencyMs ?? 0,
                    mode: t(`settings.memoryV2.recall.${lastRecall.mode}`),
                  })
                : t("settings.memoryV2.recall.noRecent")}
            </p>
          </div>
        </div>

        <div className="rounded-lg border border-border/60 bg-card p-3">
          <div className="flex gap-2.5">
            <Sparkles className="mt-0.5 h-4 w-4 shrink-0 text-primary" />
            <div className="min-w-0 flex-1">
              <div className="text-sm font-medium">{t("settings.memoryV2.learning.title")}</div>
              <p className="mt-1 text-xs leading-5 text-muted-foreground">
                {t("settings.memoryV2.learning.desc")}
              </p>
            </div>
          </div>
          <div className="mt-3 text-[11px] text-muted-foreground">
            <label htmlFor="memory-learning-mode">
            {t("settings.memoryV2.learning.mode")}
            </label>
            <Select
              value={learningMode}
              disabled={!config.enabled || saving}
              onValueChange={(value) => {
                const mode = value as MemoryLearningMode
                update((draft) => { draft.learning.mode = mode })
              }}
            >
              <SelectTrigger id="memory-learning-mode" className="mt-1 h-8 w-full text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="smart">{t("settings.memoryV2.learning.smart")}</SelectItem>
                <SelectItem value="review_first">{t("settings.memoryV2.learning.reviewFirst")}</SelectItem>
                <SelectItem value="manual">{t("settings.memoryV2.learning.manual")}</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="mt-3 flex items-center justify-between gap-2 text-[11px] text-muted-foreground">
            <span>
              {t("settings.memoryV2.learning.pending", { count: pendingCount })} ·{" "}
              {t("settings.memoryV2.learning.unassigned", { count: unassignedCount })}
            </span>
            <Button type="button" size="sm" variant="ghost" className="h-7 px-2 text-[11px]" onClick={() => void load()}>
              <RefreshCw className="mr-1 h-3 w-3" />
              {t("common.refresh")}
            </Button>
          </div>
        </div>
      </div>
    </section>
  )
}
