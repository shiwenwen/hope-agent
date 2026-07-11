import { useState, useEffect, useCallback } from "react"
import { useTranslation } from "react-i18next"
import { getTransport } from "@/lib/transport-provider"
import { Textarea } from "@/components/ui/textarea"
import { DeferredNumberInput } from "@/components/ui/deferred-number-input"
import { Switch } from "@/components/ui/switch"
import { Button } from "@/components/ui/button"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { AlertCircle, Loader2, Check, Save, Sparkles } from "lucide-react"
import ExtractConfig from "@/components/settings/memory-panel/ExtractConfig"
import MemoryFormView from "@/components/settings/memory-panel/MemoryFormView"
import MemoryListView from "@/components/settings/memory-panel/MemoryListView"
import MemoryBudgetInputs from "@/components/settings/memory-panel/MemoryBudgetInputs"
import { useMemoryData } from "@/components/settings/memory-panel/useMemoryData"
import { logger } from "@/lib/logger"
import { toast } from "sonner"
import type {
  AgentConfig,
  ActiveMemoryConfig,
  GraphMemoryConfig,
  MemoryBudgetConfig,
  ProcedureMemoryConfig,
  RetrievalPlannerConfig,
} from "../types"
import {
  DEFAULT_ACTIVE_MEMORY,
  DEFAULT_GRAPH_MEMORY,
  DEFAULT_MEMORY_BUDGET,
  DEFAULT_PROCEDURE_MEMORY,
  DEFAULT_RETRIEVAL_PLANNER,
} from "../types"
import {
  DEFAULT_AGENT_MEMORY,
  isRecommendedActiveMemory,
  RECOMMENDED_ACTIVE_MEMORY,
} from "../activeMemoryPreset"
import {
  coreMemoryOperationErrorToast,
  type CoreMemoryOperationErrorToast,
} from "../../memory-panel/coreMemoryOperationFeedback"

interface MemoryTabProps {
  agentId: string
  openclawMode?: boolean
  config: AgentConfig
  updateConfig: (patch: Partial<AgentConfig>) => void
}

export default function MemoryTab({ agentId, openclawMode, config, updateConfig }: MemoryTabProps) {
  const { t } = useTranslation()
  const memoryData = useMemoryData({ agentId, isAgentMode: true })
  const [tab, setTab] = useState<"settings" | "manage">("settings")
  const [content, setContent] = useState("")
  const [originalContent, setOriginalContent] = useState("")
  const [loaded, setLoaded] = useState(false)
  const [loading, setLoading] = useState(false)
  const [loadError, setLoadError] = useState<CoreMemoryOperationErrorToast | null>(null)
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")

  const loadContent = useCallback(async () => {
    setLoading(true)
    try {
      const md = await getTransport().call<string | null>("get_agent_memory_md", { id: agentId })
      const val = md ?? ""
      setContent(val)
      setOriginalContent(val)
      setLoaded(true)
      setLoadError(null)
    } catch (e) {
      logger.error("settings", "MemoryTab::loadCoreMemory", "Failed to load", e)
      setLoadError(coreMemoryOperationErrorToast("loadAgent", t, e))
    } finally {
      setLoading(false)
    }
  }, [agentId, t])

  useEffect(() => {
    setContent("")
    setOriginalContent("")
    setLoaded(false)
    setLoadError(null)
    setSaveStatus("idle")
    loadContent()
  }, [loadContent])

  // Listen for updates from the agent tool
  useEffect(() => {
    const unlisten = getTransport().listen("core_memory_updated", (raw) => {
      const payload = raw as { agentId: string; scope: string }
      if (payload.scope === "agent" && payload.agentId === agentId) {
        loadContent()
      }
    })
    return unlisten
  }, [agentId, loadContent])

  const handleSave = async () => {
    setSaving(true)
    try {
      await getTransport().call("save_agent_memory_md", { id: agentId, content })
      setOriginalContent(content)
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "MemoryTab::saveCoreMemory", "Failed to save", e)
      setSaveStatus("failed")
      const failureToast = coreMemoryOperationErrorToast("saveAgent", t, e)
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
      setTimeout(() => setSaveStatus("idle"), 2000)
    } finally {
      setSaving(false)
    }
  }

  const hasChanges = content !== originalContent

  const activeMemory: ActiveMemoryConfig =
    config.memory?.activeMemory ?? { ...DEFAULT_ACTIVE_MEMORY }

  const updateActiveMemory = (patch: Partial<ActiveMemoryConfig>) => {
    const prevMemory = { ...DEFAULT_AGENT_MEMORY, ...(config.memory ?? {}) }
    updateConfig({
      memory: {
        ...prevMemory,
        activeMemory: { ...activeMemory, ...patch },
      },
    })
  }
  const applyRecommendedActiveMemory = () => updateActiveMemory(RECOMMENDED_ACTIVE_MEMORY)
  const activeMemoryUsesRecommended = isRecommendedActiveMemory(activeMemory)

  const procedureMemory: ProcedureMemoryConfig =
    config.memory?.procedureMemory ?? { ...DEFAULT_PROCEDURE_MEMORY }
  const updateProcedureMemory = (patch: Partial<ProcedureMemoryConfig>) => {
    const prevMemory = { ...DEFAULT_AGENT_MEMORY, ...(config.memory ?? {}) }
    updateConfig({
      memory: {
        ...prevMemory,
        procedureMemory: { ...procedureMemory, ...patch },
      },
    })
  }

  const graphMemory: GraphMemoryConfig =
    config.memory?.graphMemory ?? { ...DEFAULT_GRAPH_MEMORY }
  const updateGraphMemory = (patch: Partial<GraphMemoryConfig>) => {
    const prevMemory = { ...DEFAULT_AGENT_MEMORY, ...(config.memory ?? {}) }
    updateConfig({
      memory: {
        ...prevMemory,
        graphMemory: { ...graphMemory, ...patch },
      },
    })
  }

  const retrievalPlanner: RetrievalPlannerConfig =
    config.memory?.retrievalPlanner ?? { ...DEFAULT_RETRIEVAL_PLANNER }
  const updateRetrievalPlanner = (patch: Partial<RetrievalPlannerConfig>) => {
    const prevMemory = { ...DEFAULT_AGENT_MEMORY, ...(config.memory ?? {}) }
    updateConfig({
      memory: {
        ...prevMemory,
        retrievalPlanner: { ...retrievalPlanner, ...patch },
      },
    })
  }

  const useGlobalBudget = !config.memory?.budget
  const budgetValue: MemoryBudgetConfig = config.memory?.budget ?? { ...DEFAULT_MEMORY_BUDGET }

  const updateMemoryBudget = (next: MemoryBudgetConfig | null) => {
    const prevMemory = { ...DEFAULT_AGENT_MEMORY, ...(config.memory ?? {}) }
    updateConfig({
      memory: {
        ...prevMemory,
        budget: next,
      },
    })
  }

  if (memoryData.view === "add" || memoryData.view === "edit") {
    return <MemoryFormView data={memoryData} />
  }

  return (
    <div className="w-full">
      <Tabs
        value={tab}
        onValueChange={(value) => setTab(value as "settings" | "manage")}
        className="w-full"
      >
        <div className="px-6 pt-2 shrink-0">
          <TabsList>
            <TabsTrigger value="settings">{t("settings.memoryTabs.settings")}</TabsTrigger>
            <TabsTrigger value="manage">{t("settings.memoryTabs.manage")}</TabsTrigger>
          </TabsList>
        </div>

        <TabsContent value="settings" className="px-6 pb-6 outline-none">
          <div className="w-full space-y-4 pt-4">
            {/* Active Memory (Phase B1) */}
            <div className="rounded-lg border border-border/60 bg-secondary/20 p-4 space-y-3">
              <div className="flex items-center justify-between gap-3">
                <div className="flex flex-col pr-4">
                  <label className="text-sm font-semibold">
                    {t("settings.activeMemoryTitle")}
                  </label>
                  <p className="text-[11px] text-muted-foreground/70 mt-0.5">
                    {t("settings.activeMemoryDesc")}
                  </p>
                </div>
                <div className="flex shrink-0 items-center gap-2">
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="h-8 gap-1.5 text-xs"
                    onClick={applyRecommendedActiveMemory}
                    disabled={activeMemoryUsesRecommended}
                  >
                    {activeMemoryUsesRecommended ? (
                      <Check className="h-3.5 w-3.5" />
                    ) : (
                      <Sparkles className="h-3.5 w-3.5" />
                    )}
                    {activeMemoryUsesRecommended
                      ? t("planMode.question.recommended")
                      : t("settings.activeMemoryUseRecommended")}
                  </Button>
                  <Switch
                    checked={activeMemory.enabled}
                    onCheckedChange={(v) => updateActiveMemory({ enabled: v })}
                  />
                </div>
              </div>
              {activeMemory.enabled && (
                <div className="grid grid-cols-2 gap-3 pt-1">
                  <label className="flex flex-col gap-1 text-xs">
                    <span className="text-muted-foreground">
                      {t("settings.activeMemoryTimeout")}
                    </span>
                    <DeferredNumberInput
                      min={200}
                      max={15000}
                      step={100}
                      className="h-8 text-xs"
                      value={activeMemory.timeoutMs}
                      onValueCommit={(value) => updateActiveMemory({ timeoutMs: value })}
                    />
                  </label>
                  <label className="flex flex-col gap-1 text-xs">
                    <span className="text-muted-foreground">
                      {t("settings.activeMemoryCacheTtl")}
                    </span>
                    <DeferredNumberInput
                      min={0}
                      max={600}
                      className="h-8 text-xs"
                      value={activeMemory.cacheTtlSecs}
                      onValueCommit={(value) => updateActiveMemory({ cacheTtlSecs: value })}
                    />
                  </label>
                  <label className="flex flex-col gap-1 text-xs">
                    <span className="text-muted-foreground">
                      {t("settings.activeMemoryMaxChars")}
                    </span>
                    <DeferredNumberInput
                      min={40}
                      max={2000}
                      className="h-8 text-xs"
                      value={activeMemory.maxChars}
                      onValueCommit={(value) => updateActiveMemory({ maxChars: value })}
                    />
                  </label>
                  <label className="flex flex-col gap-1 text-xs">
                    <span className="text-muted-foreground">
                      {t("settings.activeMemoryCandidateLimit")}
                    </span>
                    <DeferredNumberInput
                      min={1}
                      max={100}
                      className="h-8 text-xs"
                      value={activeMemory.candidateLimit}
                      onValueCommit={(value) => updateActiveMemory({ candidateLimit: value })}
                    />
                  </label>
                  <div className="col-span-2 flex items-center justify-between pt-1">
                    <div className="flex flex-col pr-4">
                      <span className="text-muted-foreground">
                        {t("settings.activeMemoryIncludeClaims")}
                      </span>
                      <p className="text-[10px] text-muted-foreground/60 mt-0.5">
                        {t("settings.activeMemoryIncludeClaimsDesc")}
                      </p>
                    </div>
                    <Switch
                      checked={activeMemory.includeClaims}
                      onCheckedChange={(v) => updateActiveMemory({ includeClaims: v })}
                    />
                  </div>
                </div>
              )}
            </div>

            {/* Retrieval Planner cross-source candidate fusion */}
            <div className="rounded-lg border border-border/60 bg-secondary/20 p-4 space-y-3">
              <div className="flex items-center justify-between gap-3">
                <div className="flex flex-col pr-4">
                  <label className="text-sm font-semibold">
                    {t("settings.retrievalPlannerTitle", "Cross-source ranking")}
                  </label>
                  <p className="text-[11px] text-muted-foreground/70 mt-0.5">
                    {t(
                      "settings.retrievalPlannerDesc",
                      "Fuse memory, relationship, workflow, and knowledge candidates with stable scope-aware ranking.",
                    )}
                  </p>
                </div>
                <div className="flex shrink-0 items-center gap-2">
                  <span className="text-xs text-muted-foreground">
                    {t("settings.retrievalPlannerIntentAware", "Task-aware")}
                  </span>
                  <Switch
                    checked={retrievalPlanner.intentAware}
                    onCheckedChange={(value) => updateRetrievalPlanner({ intentAware: value })}
                  />
                </div>
              </div>
              <div className="grid grid-cols-2 gap-3 pt-1">
                <label className="flex flex-col gap-1 text-xs">
                  <span className="text-muted-foreground">
                    {t("settings.retrievalPlannerMaxTraceRefs", "Diagnostic references")}
                  </span>
                  <DeferredNumberInput
                    min={8}
                    max={64}
                    className="h-8 text-xs"
                    value={retrievalPlanner.maxTraceRefs}
                    onValueCommit={(value) => updateRetrievalPlanner({ maxTraceRefs: value })}
                  />
                </label>
                <label className="flex flex-col gap-1 text-xs">
                  <span className="text-muted-foreground">
                    {t("settings.retrievalPlannerMaxPerOrigin", "Candidates per source")}
                  </span>
                  <DeferredNumberInput
                    min={1}
                    max={16}
                    className="h-8 text-xs"
                    value={retrievalPlanner.maxCandidatesPerOrigin}
                    onValueCommit={(value) =>
                      updateRetrievalPlanner({ maxCandidatesPerOrigin: value })
                    }
                  />
                </label>
              </div>
            </div>

            {/* Graph Memory Trace (P4) */}
            <div className="rounded-lg border border-border/60 bg-secondary/20 p-4 space-y-3">
              <div className="flex items-center justify-between gap-3">
                <div className="flex flex-col pr-4">
                  <label className="text-sm font-semibold">
                    {t("settings.graphMemoryTitle", "Entity relationship trace")}
                  </label>
                  <p className="text-[11px] text-muted-foreground/70 mt-0.5">
                    {t(
                      "settings.graphMemoryDesc",
                      "Show related structured-memory claims in answer diagnostics without injecting them into the prompt.",
                    )}
                  </p>
                </div>
                <Switch
                  checked={graphMemory.enabled}
                  onCheckedChange={(v) => updateGraphMemory({ enabled: v })}
                />
              </div>
              {graphMemory.enabled && (
                <div className="grid grid-cols-2 gap-3 pt-1">
                  <label className="flex flex-col gap-1 text-xs">
                    <span className="text-muted-foreground">
                      {t("settings.graphMemoryMaxCenters", "Center claims")}
                    </span>
                    <DeferredNumberInput
                      min={1}
                      max={8}
                      className="h-8 text-xs"
                      value={graphMemory.maxCenters}
                      onValueCommit={(value) => updateGraphMemory({ maxCenters: value })}
                    />
                  </label>
                  <label className="flex flex-col gap-1 text-xs">
                    <span className="text-muted-foreground">
                      {t("settings.graphMemoryMaxEdges", "Related claims")}
                    </span>
                    <DeferredNumberInput
                      min={1}
                      max={20}
                      className="h-8 text-xs"
                      value={graphMemory.maxEdges}
                      onValueCommit={(value) => updateGraphMemory({ maxEdges: value })}
                    />
                  </label>
                </div>
              )}
            </div>

            {/* Procedure Memory (P5) */}
            <div className="rounded-lg border border-border/60 bg-secondary/20 p-4 space-y-3">
              <div className="flex items-center justify-between gap-3">
                <div className="flex flex-col pr-4">
                  <label className="text-sm font-semibold">
                    {t("settings.procedureMemoryTitle", "Saved workflows")}
                  </label>
                  <p className="text-[11px] text-muted-foreground/70 mt-0.5">
                    {t(
                      "settings.procedureMemoryDesc",
                      "Use relevant saved workflows as bounded soft guidance before the reply.",
                    )}
                  </p>
                </div>
                <Switch
                  checked={procedureMemory.enabled}
                  onCheckedChange={(v) => updateProcedureMemory({ enabled: v })}
                />
              </div>
              {procedureMemory.enabled && (
                <div className="grid grid-cols-3 gap-3 pt-1">
                  <label className="flex flex-col gap-1 text-xs">
                    <span className="text-muted-foreground">
                      {t("settings.procedureMemoryMaxProcedures", "Workflows")}
                    </span>
                    <DeferredNumberInput
                      min={1}
                      max={3}
                      className="h-8 text-xs"
                      value={procedureMemory.maxProcedures}
                      onValueCommit={(value) =>
                        updateProcedureMemory({ maxProcedures: value })
                      }
                    />
                  </label>
                  <label className="flex flex-col gap-1 text-xs">
                    <span className="text-muted-foreground">
                      {t("settings.procedureMemoryMaxChars", "Max chars")}
                    </span>
                    <DeferredNumberInput
                      min={200}
                      max={2000}
                      step={50}
                      className="h-8 text-xs"
                      value={procedureMemory.maxChars}
                      onValueCommit={(value) => updateProcedureMemory({ maxChars: value })}
                    />
                  </label>
                  <label className="flex flex-col gap-1 text-xs">
                    <span className="text-muted-foreground">
                      {t("settings.procedureMemoryMinConfidence", "Min confidence")}
                    </span>
                    <DeferredNumberInput
                      min={0}
                      max={100}
                      step={5}
                      className="h-8 text-xs"
                      value={Math.round(procedureMemory.minConfidence * 100)}
                      onValueCommit={(value) =>
                        updateProcedureMemory({
                          minConfidence: Math.max(0, Math.min(100, value)) / 100,
                        })
                      }
                    />
                  </label>
                </div>
              )}
            </div>

            {/* Memory Budget override (Agent level) */}
            <div className="rounded-lg border border-border/60 bg-secondary/20 p-4 space-y-3">
              <div className="flex items-center justify-between">
                <div className="flex flex-col pr-4">
                  <label className="text-sm font-semibold">
                    {t("settings.memoryBudget.title")}
                  </label>
                  <p className="text-[11px] text-muted-foreground/70 mt-0.5">
                    {t("settings.memoryBudget.agentOverrideDesc")}
                  </p>
                </div>
                <div className="flex items-center gap-2">
                  <span className="text-xs text-muted-foreground">
                    {t("settings.memoryBudget.useGlobalDefault")}
                  </span>
                  <Switch
                    checked={useGlobalBudget}
                    onCheckedChange={(v) =>
                      updateMemoryBudget(v ? null : { ...DEFAULT_MEMORY_BUDGET })
                    }
                  />
                </div>
              </div>
              <MemoryBudgetInputs
                value={budgetValue}
                onChange={(next) => updateMemoryBudget(next)}
                disabled={useGlobalBudget}
              />
            </div>

            <ExtractConfig data={memoryData} isAgentMode />
          </div>
        </TabsContent>

        <TabsContent value="manage" className="p-6 outline-none">
          {/* Core Memory Editor */}
          <div className="mb-4 w-full">
            <div className="flex items-center justify-between mb-1">
              <div className="flex items-center gap-1.5">
                <h3 className="text-sm font-semibold">{t("settings.coreMemory")}</h3>
                {loading && <Loader2 className="h-3 w-3 animate-spin text-muted-foreground" />}
              </div>
              {loaded && (
                <Button
                  size="sm"
                  className="gap-1.5 h-7 text-xs"
                  disabled={saving || !hasChanges}
                  onClick={handleSave}
                  variant={saveStatus === "saved" ? "outline" : saveStatus === "failed" ? "destructive" : "default"}
                >
                  {saving ? (
                    <><Loader2 className="h-3 w-3 animate-spin" />{t("common.saving")}</>
                  ) : saveStatus === "saved" ? (
                    <><Check className="h-3 w-3" />{t("common.saved")}</>
                  ) : (
                    <><Save className="h-3 w-3" />{t("common.save")}</>
                  )}
                </Button>
              )}
            </div>
            <p className="text-xs text-muted-foreground mb-3">{t("settings.coreMemoryAgentDesc")}</p>
            {openclawMode && (
              <div className="rounded-lg border border-green-500/30 bg-green-500/5 px-3 py-2 mb-3">
                <p className="text-xs text-green-600 dark:text-green-400">
                  {t("settings.openclawMemoryHint")}
                </p>
              </div>
            )}
            {loadError && (
              <div className="mb-3 rounded-md border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs">
                <div className="flex items-center gap-1.5 font-medium text-foreground">
                  <AlertCircle className="h-3.5 w-3.5 text-amber-500" />
                  {loadError.title}
                </div>
                {loadError.description && (
                  <div className="mt-1 break-all text-muted-foreground">
                    {loadError.description}
                  </div>
                )}
                <button
                  type="button"
                  className="mt-2 font-medium text-foreground underline underline-offset-2"
                  onClick={() => void loadContent()}
                >
                  {t("common.retry", "Retry")}
                </button>
              </div>
            )}
            {loaded && (
              <Textarea
                value={content}
                onChange={(e) => setContent(e.target.value)}
                placeholder={t("settings.coreMemoryPlaceholder")}
                className="min-h-[100px] max-h-[200px] text-sm font-mono resize-y"
              />
            )}
          </div>

          <MemoryListView data={memoryData} isAgentMode compact embedded />
        </TabsContent>
      </Tabs>
    </div>
  )
}
