import { useCallback, useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import {
  AlertCircle,
  AlertTriangle,
  Brain,
  Check,
  CheckCircle2,
  ChevronDown,
  Loader2,
  RotateCcw,
  Settings2,
  StopCircle,
  Zap,
} from "lucide-react"

import { AnimatedCollapse } from "@/components/ui/animated-presence"
import { Button } from "@/components/ui/button"
import { NumberInput } from "@/components/ui/number-input"
import { DeferredNumberInput } from "@/components/ui/deferred-number-input"
import { Progress } from "@/components/ui/progress"
import { Switch } from "@/components/ui/switch"
import { ModelChainEditor, type ModelChainRef } from "@/components/ui/model-chain-editor"
import type { AvailableModel } from "@/components/ui/model-selector"
import { getTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"
import { logger } from "@/lib/logger"
import type {
  ChunkConfig,
  KnowledgeCompileConfig,
  KnowledgeMediaRetentionConfig,
  KnowledgeSearchConfig,
  KnowledgeVisionConfig,
  NoteToolsConfig,
  PassiveRecallConfig,
} from "@/types/knowledge"
import {
  isLocalModelJobActive,
  isLocalModelJobTerminal,
  phaseTranslationKey,
  type LocalModelJobSnapshot,
} from "@/types/local-model-jobs"
import { useReembedJob } from "@/hooks/useReembedJob"
import {
  embeddingProviderLabel,
  type EmbeddingModelConfig,
  // The wire shape of `knowledge_embedding_get_cmd` is `EmbeddingSelectionState`
  // (generic), serialized identically to the memory one — reuse the TS type.
  type MemoryEmbeddingState as EmbeddingSelectionState,
} from "@/types/embedding-models"
import EmbeddingActivationDialog from "./memory-panel/EmbeddingActivationDialog"
import KnowledgeMaintenanceSection from "./KnowledgeMaintenanceSection"
import SpriteSection from "./SpriteSection"
import KnowledgeSourceLimitsSection from "./KnowledgeSourceLimitsSection"
import {
  knowledgeChunkOperationErrorToast,
  knowledgeCompileAgentOperationErrorToast,
  knowledgeMediaRetentionOperationErrorToast,
  knowledgePassiveRecallOperationErrorToast,
  knowledgePanelErrorDetail,
  knowledgePanelOperationErrorToast,
  knowledgeSearchRankingOperationErrorToast,
  type KnowledgePanelOperationErrorToast,
} from "./knowledgePanelFeedback"
import SettingsResetControl from "./SettingsResetControl"


const EMPTY_STATE: EmbeddingSelectionState = {
  selection: { enabled: false, modelConfigId: null, activeSignature: null, lastReembeddedSignature: null },
  currentModel: null,
  needsReembed: false,
}

const DEFAULT_KNOWLEDGE_VISION_CONFIG: KnowledgeVisionConfig = {
  modelOverride: null,
  timeoutSecs: 90,
  maxTokens: 4096,
  ocrConcurrency: 3,
  maxOcrPages: 40,
}

/**
 * Knowledge-base vector-search (embedding) settings (D7).
 *
 * Independent of memory: own enable switch / model / signature / reembed
 * lifecycle, but draws from the **shared** `embedding_models` library (managed
 * under Settings → Memory → Embedding models). When the library is empty the
 * activation dialog collapses to a "go configure / download a local model" CTA,
 * reusing memory's onboarding path.
 */
export default function KnowledgePanel() {
  const { t } = useTranslation()
  const [state, setState] = useState<EmbeddingSelectionState>(EMPTY_STATE)
  const [models, setModels] = useState<EmbeddingModelConfig[]>([])
  const [activationOpen, setActivationOpen] = useState(false)
  const [loadError, setLoadError] = useState<KnowledgePanelOperationErrorToast | null>(null)
  const [sectionRevisions, setSectionRevisions] = useState({
    compile: 0,
    vision: 0,
    noteTools: 0,
  })

  const refreshSection = (section: keyof typeof sectionRevisions) => {
    setSectionRevisions((current) => ({ ...current, [section]: current[section] + 1 }))
  }

  const reload = useCallback(async () => {
    try {
      const [list, st] = await Promise.all([
        getTransport().call<EmbeddingModelConfig[]>("embedding_model_config_list"),
        getTransport().call<EmbeddingSelectionState>("knowledge_embedding_get_cmd"),
      ])
      setModels(list)
      setState(st)
      setLoadError(null)
    } catch (e) {
      logger.error("settings", "KnowledgePanel::reload", "Failed to load knowledge embedding", e)
      setLoadError(knowledgePanelOperationErrorToast("loadEmbedding", t, e))
    }
  }, [t])

  // Initial load. setState lives in the async `.then` callback (not the effect
  // body) per react-hooks/set-state-in-effect; `reload` is reused by the reembed
  // hook's onCompleted, where calling it is fine (it's a listener callback).
  useEffect(() => {
    let cancelled = false
    Promise.all([
      getTransport().call<EmbeddingModelConfig[]>("embedding_model_config_list"),
      getTransport().call<EmbeddingSelectionState>("knowledge_embedding_get_cmd"),
    ])
      .then(([list, st]) => {
        if (cancelled) return
        setModels(list)
        setState(st)
        setLoadError(null)
      })
      .catch((e) => {
        logger.error("settings", "KnowledgePanel::load", "Failed to load knowledge embedding", e)
        if (!cancelled) setLoadError(knowledgePanelOperationErrorToast("loadEmbedding", t, e))
      })
    return () => {
      cancelled = true
    }
  }, [t])

  // Track the knowledge reembed job via the shared hook (same plumbing as the
  // memory panel; only the kind filter + onCompleted differ). Refresh state on
  // completion so `needsReembed` clears.
  const { job: reembedJob } = useReembedJob({ kind: "knowledge_reembed", onCompleted: reload })

  const activate = useCallback(
    async (modelConfigId: string): Promise<boolean> => {
      try {
        const st = await getTransport().call<EmbeddingSelectionState>(
          "knowledge_embedding_set_default_cmd",
          { modelConfigId },
        )
        setState(st)
        toast.success(t("settings.knowledgeEmbedding.activated"))
        return true
      } catch (e) {
        logger.error("settings", "KnowledgePanel::activate", "Failed to activate", e)
        const failure = knowledgePanelOperationErrorToast("activateEmbedding", t, e)
        toast.error(
          failure.title,
          failure.description ? { description: failure.description } : undefined,
        )
        return false
      }
    },
    [t],
  )

  function handleToggle(next: boolean) {
    if (!next) {
      void getTransport()
        .call<EmbeddingSelectionState>("knowledge_embedding_disable_cmd")
        .then(setState)
        .catch((e) => {
          logger.error("settings", "KnowledgePanel::disable", "Failed to disable", e)
          const failure = knowledgePanelOperationErrorToast("disableEmbedding", t, e)
          toast.error(
            failure.title,
            failure.description ? { description: failure.description } : undefined,
          )
        })
      return
    }
    const remembered = state.selection.modelConfigId
    const stillValid = remembered && models.some((m) => m.id === remembered)
    if (stillValid) {
      void activate(remembered)
    } else {
      setActivationOpen(true)
    }
  }

  // Force a full rebuild of every KB — bypasses set_default's same-signature
  // short-circuit so the button always works; progress shows in the card below.
  const rebuild = useCallback(async () => {
    try {
      await getTransport().call("knowledge_embedding_rebuild_cmd")
    } catch (e) {
      logger.error("settings", "KnowledgePanel::rebuild", "Failed to start rebuild", e)
      const failure = knowledgePanelOperationErrorToast("rebuildEmbedding", t, e)
      toast.error(
        failure.title,
        failure.description ? { description: failure.description } : undefined,
      )
    }
  }, [t])

  const enabled = state.selection.enabled
  const current = state.currentModel

  return (
    <div className="flex-1 overflow-y-auto p-6 space-y-4">
      <div>
        <h2 className="text-lg font-semibold">{t("settings.knowledgeEmbedding.title")}</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          {t("settings.knowledgeEmbedding.description")}
        </p>
      </div>

      <CompileAgentSection
        key={sectionRevisions.compile}
        onReset={() => refreshSection("compile")}
      />

      <KnowledgeVisionSection
        key={sectionRevisions.vision}
        onReset={() => refreshSection("vision")}
      />

      <NoteToolsSection
        key={sectionRevisions.noteTools}
        onReset={() => refreshSection("noteTools")}
      />

      <div className="flex items-center justify-between rounded-lg bg-secondary/30 px-3 py-3">
        <div>
          <div className="text-sm font-medium">{t("settings.knowledgeEmbedding.enabled")}</div>
          <div className="text-xs text-muted-foreground">
            {t("settings.knowledgeEmbedding.enabledDesc")}
          </div>
        </div>
        <Switch checked={enabled} onCheckedChange={handleToggle} />
      </div>

      {loadError && (
        <div className="flex items-start gap-2 rounded-md border border-amber-500/25 bg-amber-500/10 px-3 py-2 text-xs text-amber-800 dark:text-amber-200">
          <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <div className="min-w-0 flex-1">
            <div className="font-medium">{loadError.title}</div>
            {loadError.description ? (
              <div className="mt-0.5 whitespace-pre-wrap text-amber-800/80 dark:text-amber-100/80">
                {loadError.description}
              </div>
            ) : null}
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="mt-1 h-7 px-2 text-xs"
              onClick={() => void reload()}
            >
              <RotateCcw className="mr-1.5 h-3.5 w-3.5" />
              {t("common.retry", "Retry")}
            </Button>
          </div>
        </div>
      )}

      {enabled && current && (
        <div className="flex items-start gap-3 rounded-lg border border-border bg-card px-3 py-2.5">
          <Brain className="mt-0.5 h-4 w-4 shrink-0 text-primary" />
          <div className="min-w-0 flex-1">
            <div className="text-sm font-medium">{current.name}</div>
            <div className="mt-0.5 text-xs text-muted-foreground">
              {embeddingProviderLabel(current)}
              {current.apiModel ? ` · ${current.apiModel}` : ""}
              {current.apiDimensions ? ` · ${current.apiDimensions}d` : ""}
            </div>
          </div>
          <Button variant="outline" size="sm" onClick={() => setActivationOpen(true)}>
            {t("settings.knowledgeEmbedding.changeModel")}
          </Button>
        </div>
      )}

      {enabled && (
        <KnowledgeReembedCard
          job={reembedJob}
          needsReembed={state.needsReembed}
          canRebuild={!!current}
          onRebuild={() => void rebuild()}
        />
      )}

      <ChunkAdvancedSection />

      <SearchRankingSection />

      <PassiveRecallSection />

      <KnowledgeSourceLimitsSection />

      <MediaRetentionSection />

      <KnowledgeMaintenanceSection />

      <SpriteSection />

      <EmbeddingActivationDialog
        open={activationOpen}
        onOpenChange={setActivationOpen}
        embeddingModels={models}
        onConfirm={activate}
      />
    </div>
  )
}

function CompileAgentSection({ onReset }: { onReset: () => void }) {
  const { t } = useTranslation()
  const [config, setConfig] = useState<KnowledgeCompileConfig>({ modelOverride: null })
  const [availableModels, setAvailableModels] = useState<AvailableModel[]>([])
  const [loaded, setLoaded] = useState(false)
  const [saving, setSaving] = useState(false)
  const [loadIssues, setLoadIssues] = useState<KnowledgePanelOperationErrorToast[]>([])
  const confirmedConfigRef = useRef<KnowledgeCompileConfig>({ modelOverride: null })

  useEffect(() => {
    let cancelled = false
    async function load() {
      const issues: KnowledgePanelOperationErrorToast[] = []
      const [configResult, modelsResult] = await Promise.allSettled([
        getTransport().call<KnowledgeCompileConfig>(
          "knowledge_compile_config_get_cmd",
        ),
        getTransport().call<AvailableModel[]>("get_available_models"),
      ])

      if (cancelled) return

      if (configResult.status === "fulfilled") {
        const loadedConfig = { modelOverride: configResult.value?.modelOverride ?? null }
        confirmedConfigRef.current = loadedConfig
        setConfig(loadedConfig)
      } else {
        logger.error(
          "settings",
          "KnowledgePanel::compileAgentLoad",
          "Failed to load",
          configResult.reason,
        )
        issues.push(
          knowledgeCompileAgentOperationErrorToast("loadConfig", t, configResult.reason),
        )
      }

      if (modelsResult.status === "fulfilled") {
        setAvailableModels(modelsResult.value)
      } else {
        logger.error(
          "settings",
          "KnowledgePanel::compileAgentAgents",
          "Failed to load models",
          modelsResult.reason,
        )
        issues.push(
          knowledgeCompileAgentOperationErrorToast("loadAgents", t, modelsResult.reason),
        )
      }

      setLoadIssues(issues)
      setLoaded(true)
    }
    void load()
    return () => {
      cancelled = true
    }
  }, [t])

  const handleModelOverrideChange = useCallback(
    async (next: ModelChainRef | null) => {
      const nextConfig: KnowledgeCompileConfig = { modelOverride: next }
      const previous = confirmedConfigRef.current
      setConfig(nextConfig)
      setSaving(true)
      try {
        const saved = await getTransport().call<KnowledgeCompileConfig>(
          "knowledge_compile_config_set_cmd",
          { config: nextConfig },
        )
        const savedConfig = { modelOverride: saved.modelOverride ?? null }
        confirmedConfigRef.current = savedConfig
        setConfig(savedConfig)
      } catch (e) {
        setConfig(previous)
        logger.error("settings", "KnowledgePanel::compileAgentSave", "Failed to save", e)
        const failure = knowledgeCompileAgentOperationErrorToast("saveAgent", t, e)
        toast.error(
          failure.title,
          failure.description ? { description: failure.description } : undefined,
        )
      } finally {
        setSaving(false)
      }
    },
    [t],
  )

  if (!loaded) return null

  return (
    <div className="rounded-lg bg-secondary/30 px-3 py-3">
      <div className="mb-2 flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="text-sm font-medium">{t("settings.knowledgeCompile.agent")}</div>
          <div className="text-xs text-muted-foreground">
            {t("settings.knowledgeCompile.agentDesc")}
          </div>
        </div>
        <SettingsResetControl
          scope="knowledge"
          resetSection="compile"
          sectionLabel={t("settings.knowledgeCompile.agent")}
          level="region"
          onReset={onReset}
        />
      </div>
      <ModelChainEditor
        value={config.modelOverride ?? null}
        onChange={(next) => void handleModelOverrideChange(next)}
        availableModels={availableModels}
        inheritLabel={t("settings.knowledgeCompile.agentDefault")}
        className={saving ? "pointer-events-none opacity-60" : undefined}
      />
      {loadIssues.length > 0 && (
        <div className="mt-2 space-y-1.5">
          {loadIssues.map((issue) => (
            <div
              key={issue.title}
              className="rounded-md border border-amber-500/25 bg-amber-500/10 px-2.5 py-1.5 text-[11px] text-amber-800 dark:text-amber-200"
            >
              <div className="flex items-center gap-1.5 font-medium">
                <AlertCircle className="h-3 w-3 shrink-0" />
                <span>{issue.title}</span>
              </div>
              {issue.description ? (
                <div className="mt-0.5 whitespace-pre-wrap text-amber-800/80 dark:text-amber-100/80">
                  {issue.description}
                </div>
              ) : null}
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

function KnowledgeVisionSection({ onReset }: { onReset: () => void }) {
  const { t } = useTranslation()
  const [config, setConfig] = useState<KnowledgeVisionConfig>(DEFAULT_KNOWLEDGE_VISION_CONFIG)
  const [availableModels, setAvailableModels] = useState<AvailableModel[]>([])
  const [loaded, setLoaded] = useState(false)

  useEffect(() => {
    let cancelled = false
    Promise.all([
      getTransport().call<KnowledgeVisionConfig>("knowledge_vision_config_get_cmd"),
      getTransport().call<AvailableModel[]>("get_available_models").catch(() => []),
    ])
      .then(([cfg, models]) => {
        if (cancelled) return
        setConfig(
          cfg
            ? {
                ...DEFAULT_KNOWLEDGE_VISION_CONFIG,
                ...cfg,
                modelOverride: cfg.modelOverride ?? null,
              }
            : DEFAULT_KNOWLEDGE_VISION_CONFIG,
        )
        setAvailableModels(models ?? [])
        setLoaded(true)
      })
      .catch((e) => {
        logger.error("settings", "KnowledgePanel::visionLoad", "Failed to load", e)
        setLoaded(true)
      })
    return () => {
      cancelled = true
    }
  }, [])

  const saveConfig = useCallback(async (nextConfig: KnowledgeVisionConfig) => {
    setConfig(nextConfig)
    try {
      const saved = await getTransport().call<KnowledgeVisionConfig>(
        "knowledge_vision_config_set_cmd",
        { config: nextConfig },
      )
      setConfig(saved)
    } catch (e) {
      logger.error("settings", "KnowledgePanel::visionSave", "Failed to save", e)
      toast.error(String(e))
    }
  }, [])

  const handleModelOverrideChange = useCallback(
    (next: ModelChainRef | null) => void saveConfig({ ...config, modelOverride: next }),
    [config, saveConfig],
  )

  if (!loaded) return null

  return (
    <div className="rounded-lg bg-secondary/30 px-3 py-3">
      <div className="mb-2 flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="text-sm font-medium">{t("settings.knowledgeVision.model")}</div>
          <div className="text-xs text-muted-foreground">
            {t("settings.knowledgeVision.modelDesc")}
          </div>
        </div>
        <SettingsResetControl
          scope="knowledge"
          resetSection="vision"
          sectionLabel={t("settings.knowledgeVision.model")}
          level="region"
          onReset={onReset}
        />
      </div>
      <ModelChainEditor
        value={config.modelOverride ?? null}
        onChange={handleModelOverrideChange}
        availableModels={availableModels}
        inheritLabel={t("settings.knowledgeVision.modelDefault")}
      />
      <div className="mt-3 grid grid-cols-2 gap-3">
        <label className="space-y-1">
          <span className="text-xs font-medium">
            {t("settings.knowledgeVision.ocrConcurrency", "Scanned-PDF OCR concurrency")}
          </span>
          <DeferredNumberInput
            min={1}
            max={8}
            value={config.ocrConcurrency}
            onValueCommit={(value) => void saveConfig({ ...config, ocrConcurrency: value })}
            className="h-8 text-xs"
          />
        </label>
        <label className="space-y-1">
          <span className="text-xs font-medium">
            {t("settings.knowledgeVision.maxOcrPages", "Scanned-PDF OCR page limit")}
          </span>
          <DeferredNumberInput
            min={1}
            max={120}
            value={config.maxOcrPages}
            onValueCommit={(value) => void saveConfig({ ...config, maxOcrPages: value })}
            className="h-8 text-xs"
          />
        </label>
      </div>
    </div>
  )
}

function NoteToolsSection({ onReset }: { onReset: () => void }) {
  const { t } = useTranslation()
  const [config, setConfig] = useState<NoteToolsConfig>({ modelOverride: null })
  const [availableModels, setAvailableModels] = useState<AvailableModel[]>([])
  const [loaded, setLoaded] = useState(false)

  useEffect(() => {
    let cancelled = false
    Promise.all([
      getTransport().call<NoteToolsConfig>("note_tools_config_get_cmd"),
      getTransport().call<AvailableModel[]>("get_available_models").catch(() => []),
    ])
      .then(([cfg, models]) => {
        if (cancelled) return
        setConfig({ modelOverride: cfg?.modelOverride ?? null })
        setAvailableModels(models ?? [])
        setLoaded(true)
      })
      .catch((e) => {
        logger.error("settings", "KnowledgePanel::noteToolsLoad", "Failed to load", e)
        setLoaded(true)
      })
    return () => {
      cancelled = true
    }
  }, [])

  const handleModelOverrideChange = useCallback(async (next: ModelChainRef | null) => {
    const nextConfig: NoteToolsConfig = { modelOverride: next }
    setConfig(nextConfig)
    try {
      const saved = await getTransport().call<NoteToolsConfig>("note_tools_config_set_cmd", {
        config: nextConfig,
      })
      setConfig({ modelOverride: saved.modelOverride ?? null })
    } catch (e) {
      logger.error("settings", "KnowledgePanel::noteToolsSave", "Failed to save", e)
      toast.error(String(e))
    }
  }, [])

  if (!loaded) return null

  return (
    <div className="rounded-lg bg-secondary/30 px-3 py-3">
      <div className="mb-2 flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="text-sm font-medium">{t("settings.noteTools.title")}</div>
          <div className="text-xs text-muted-foreground">{t("settings.noteTools.desc")}</div>
        </div>
        <SettingsResetControl
          scope="knowledge"
          resetSection="note_tools"
          sectionLabel={t("settings.noteTools.title")}
          level="region"
          onReset={onReset}
        />
      </div>
      <ModelChainEditor
        value={config.modelOverride ?? null}
        onChange={(next) => void handleModelOverrideChange(next)}
        availableModels={availableModels}
        inheritLabel={t("settings.noteTools.modelDefault")}
      />
    </div>
  )
}

/**
 * Reembed status for the knowledge index. Progress is KB-granular. Visually
 * parallels memory's `ReembedJobCard` (status icon / progress bar / error /
 * retry) but with a knowledge-specific idle state (needs-rebuild / Rebuild-now)
 * vs memory's manual reembed-all + dismiss. TODO(dedup): if those idle/badge
 * differences converge, extract a shared `<ReembedJobView>`.
 */
function KnowledgeReembedCard({
  job,
  needsReembed,
  canRebuild,
  onRebuild,
}: {
  job: LocalModelJobSnapshot | null
  needsReembed: boolean
  canRebuild: boolean
  onRebuild: () => void
}) {
  const { t } = useTranslation()

  if (!job) {
    return (
      <div className="rounded-lg border border-border bg-card p-4">
        <div className="flex items-center justify-between gap-3">
          <div>
            <div className="text-sm font-medium">{t("settings.knowledgeEmbedding.reembed.title")}</div>
            <div className="mt-0.5 text-xs text-muted-foreground">
              {needsReembed
                ? t("settings.knowledgeEmbedding.reembed.needs")
                : t("settings.knowledgeEmbedding.reembed.idle")}
            </div>
          </div>
          <Button variant="outline" size="sm" disabled={!canRebuild} onClick={onRebuild}>
            <Zap className="mr-1.5 h-3.5 w-3.5" />
            {t("settings.knowledgeEmbedding.reembed.startNow")}
          </Button>
        </div>
      </div>
    )
  }

  const active = isLocalModelJobActive(job)
  const terminal = isLocalModelJobTerminal(job)
  const phaseKey = phaseTranslationKey(job.phase)
  const phaseLabel = phaseKey ? t(phaseKey) : job.phase
  const done = Number(job.bytesCompleted ?? 0)
  const total = Number(job.bytesTotal ?? 0)
  const percent = total > 0 ? Math.min(100, Math.floor((done / total) * 100)) : (job.percent ?? 0)
  const jobError = knowledgePanelErrorDetail(job.error)

  return (
    <div className="rounded-lg border border-border bg-card p-4">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 text-sm font-medium">
            {active ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin text-primary" />
            ) : job.status === "completed" ? (
              <CheckCircle2 className="h-3.5 w-3.5 text-emerald-500" />
            ) : job.status === "failed" || job.status === "interrupted" ? (
              <AlertCircle className="h-3.5 w-3.5 text-destructive" />
            ) : (
              <StopCircle className="h-3.5 w-3.5 text-muted-foreground" />
            )}
            {t("settings.knowledgeEmbedding.reembed.title")}
          </div>
          <div className="mt-1 text-xs text-muted-foreground">
            {job.displayName} · {phaseLabel}
          </div>
        </div>
        {active && (
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              void getTransport()
                .call("local_model_job_cancel", { jobId: job.jobId })
                .catch((e) => {
                  const failure = knowledgePanelOperationErrorToast("cancelReembed", t, e)
                  toast.error(
                    failure.title,
                    failure.description ? { description: failure.description } : undefined,
                  )
                })
            }}
          >
            <StopCircle className="mr-1.5 h-3.5 w-3.5" />
            {t("settings.knowledgeEmbedding.reembed.cancel")}
          </Button>
        )}
      </div>

      {(active || terminal) && (
        <div className="mt-3">
          <Progress value={percent} />
          <div className="mt-1 flex items-center justify-between text-[11px] text-muted-foreground">
            <span>{t("settings.knowledgeEmbedding.reembed.progress", { done, total })}</span>
            <span className="font-mono">{percent}%</span>
          </div>
        </div>
      )}

      {jobError && (
        <div className="mt-3 flex items-start gap-2 rounded border border-destructive/30 bg-destructive/5 px-2.5 py-2 text-xs text-destructive">
          <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <span className="break-words">{jobError}</span>
        </div>
      )}

      {terminal && (job.status === "failed" || job.status === "interrupted") && (
        <div className="mt-3 flex justify-end border-t border-border/60 pt-3">
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              void getTransport()
                .call("local_model_job_retry", { jobId: job.jobId })
                .catch((e) => {
                  const failure = knowledgePanelOperationErrorToast("retryReembed", t, e)
                  toast.error(
                    failure.title,
                    failure.description ? { description: failure.description } : undefined,
                  )
                })
            }}
          >
            <RotateCcw className="mr-1.5 h-3.5 w-3.5" />
            {t("settings.knowledgeEmbedding.reembed.retry")}
          </Button>
        </div>
      )}
    </div>
  )
}

/**
 * Read bridge ③ — passive related-notes prompt (Phase 3, D7). When enabled, each
 * user turn surfaces the top accessible-KB note titles as an untrusted reference
 * block. Enabled by default after KB access is granted; access is already
 * per-session gated so one global toggle suffices. The enable switch saves immediately; the tuning knobs
 * use the three-state Save button.
 */
function PassiveRecallSection() {
  const { t } = useTranslation()
  const [loaded, setLoaded] = useState<PassiveRecallConfig | null>(null)
  const [draft, setDraft] = useState<PassiveRecallConfig | null>(null)
  const [loadError, setLoadError] = useState<KnowledgePanelOperationErrorToast | null>(null)
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")

  const reload = useCallback(async () => {
    try {
      const c = await getTransport().call<PassiveRecallConfig>(
        "kb_passive_recall_config_get_cmd",
      )
      setLoaded(c)
      setDraft(c)
      setLoadError(null)
    } catch (e) {
      logger.error("settings", "PassiveRecallSection::reload", "Failed to load config", e)
      setLoadError(knowledgePassiveRecallOperationErrorToast("load", t, e))
    }
  }, [t])

  useEffect(() => {
    let cancelled = false
    getTransport()
      .call<PassiveRecallConfig>("kb_passive_recall_config_get_cmd")
      .then((c) => {
        if (cancelled) return
        setLoaded(c)
        setDraft(c)
        setLoadError(null)
      })
      .catch((e) => {
        logger.error("settings", "PassiveRecallSection::load", "Failed to load config", e)
        if (!cancelled) setLoadError(knowledgePassiveRecallOperationErrorToast("load", t, e))
      })
    return () => {
      cancelled = true
    }
  }, [t])

  const persist = useCallback(
    async (next: PassiveRecallConfig, viaButton: boolean) => {
      if (viaButton) setSaving(true)
      try {
        const saved = await getTransport().call<PassiveRecallConfig>(
          "kb_passive_recall_config_set_cmd",
          { config: next },
        )
        setLoaded(saved)
        if (viaButton) {
          // Save button: the draft IS what we persisted, so sync fully.
          setDraft(saved)
          setSaving(false)
          setSaveStatus("saved")
          setTimeout(() => setSaveStatus("idle"), 2000)
          toast.success(t("settings.knowledgePassiveRecall.saved", "Saved"))
        } else {
          // Enable toggle: persisted the last-saved knobs + new `enabled`, so keep
          // the user's in-progress knob edits — only reconcile `enabled`.
          setDraft((d) => (d ? { ...d, enabled: saved.enabled } : saved))
        }
      } catch (e) {
        logger.error("settings", "PassiveRecallSection::save", "Failed to save config", e)
        if (viaButton) {
          setSaving(false)
          setSaveStatus("failed")
          setTimeout(() => setSaveStatus("idle"), 2000)
        } else {
          // Revert the optimistic enable flip.
          setDraft((d) => (d ? { ...d, enabled: loaded?.enabled ?? !next.enabled } : d))
        }
        const failure = knowledgePassiveRecallOperationErrorToast(
          viaButton ? "save" : "toggle",
          t,
          e,
        )
        toast.error(
          failure.title,
          failure.description ? { description: failure.description } : undefined,
        )
      }
    },
    [loaded, t],
  )

  if (!draft || !loaded) {
    if (!loadError) return null
    return (
      <div className="rounded-lg border border-border bg-card">
        <div className="px-4 py-3">
          <div className="text-sm font-medium">
            {t("settings.knowledgePassiveRecall.title", "Passive related notes")}
          </div>
          <div className="mt-0.5 text-xs text-muted-foreground">
            {t(
              "settings.knowledgePassiveRecall.description",
              "Each turn, surface titles of accessible notes related to your message as a hint.",
            )}
          </div>
          <div className="mt-3 flex items-start gap-2 rounded-md border border-amber-500/25 bg-amber-500/10 px-3 py-2 text-xs text-amber-800 dark:text-amber-200">
            <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <div className="min-w-0 flex-1">
              <div className="font-medium">{loadError.title}</div>
              {loadError.description ? (
                <div className="mt-0.5 whitespace-pre-wrap text-amber-800/80 dark:text-amber-100/80">
                  {loadError.description}
                </div>
              ) : null}
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="mt-1 h-7 px-2 text-xs"
                onClick={() => void reload()}
              >
                <RotateCcw className="mr-1.5 h-3.5 w-3.5" />
                {t("common.retry", "Retry")}
              </Button>
            </div>
          </div>
        </div>
      </div>
    )
  }

  const enabled = draft.enabled
  const dirty =
    loaded.topN !== draft.topN ||
    loaded.maxChars !== draft.maxChars ||
    loaded.cacheTtlSecs !== draft.cacheTtlSecs ||
    loaded.showSnippet !== draft.showSnippet

  return (
    <div className="rounded-lg border border-border bg-card">
      <div className="flex items-center justify-between px-4 py-3">
        <div className="min-w-0">
          <div className="text-sm font-medium">
            {t("settings.knowledgePassiveRecall.title", "Passive related notes")}
          </div>
          <div className="mt-0.5 text-xs text-muted-foreground">
            {t(
              "settings.knowledgePassiveRecall.description",
              "Each turn, surface titles of accessible notes related to your message as a hint.",
            )}
          </div>
        </div>
        <div className="flex items-center gap-2">
          <SettingsResetControl
            scope="knowledge"
            resetSection="passive_recall"
            sectionLabel={t("settings.knowledgePassiveRecall.title", "Passive related notes")}
            level="region"
            onReset={reload}
          />
          <Switch
            checked={enabled}
            onCheckedChange={(v) => {
              // Optimistically flip in the draft; persist the LAST-SAVED knobs + new
              // `enabled` so toggling the master switch never silently commits the
              // user's unsaved knob edits (those stay pending behind the Save button).
              setDraft({ ...draft, enabled: v })
              void persist({ ...loaded, enabled: v }, false)
            }}
          />
        </div>
      </div>

      <AnimatedCollapse open={enabled}>
        <div className="space-y-3 border-t border-border px-4 py-3">
          <div className="grid grid-cols-3 gap-3">
            <label className="space-y-1">
              <span className="text-xs font-medium">
                {t("settings.knowledgePassiveRecall.topN", "Max notes")}
              </span>
              <DeferredNumberInput
                min={1}
                max={20}
                value={draft.topN}
                onValueCommit={(value) => setDraft({ ...draft, topN: value })}
                className="h-8 text-xs"
              />
            </label>
            <label className="space-y-1">
              <span className="text-xs font-medium">
                {t("settings.knowledgePassiveRecall.maxChars", "Max chars")}
              </span>
              <DeferredNumberInput
                min={100}
                max={4000}
                value={draft.maxChars}
                onValueCommit={(value) => setDraft({ ...draft, maxChars: value })}
                className="h-8 text-xs"
              />
            </label>
            <label className="space-y-1">
              <span className="text-xs font-medium">
                {t("settings.knowledgePassiveRecall.cacheTtlSecs", "Cache (s)")}
              </span>
              <DeferredNumberInput
                min={1}
                value={draft.cacheTtlSecs}
                onValueCommit={(value) => setDraft({ ...draft, cacheTtlSecs: value })}
                className="h-8 text-xs"
              />
            </label>
          </div>
          <div className="flex items-center justify-between">
            <div className="min-w-0 pr-3">
              <div className="text-xs font-medium">
                {t("settings.knowledgePassiveRecall.showSnippet", "Show snippet")}
              </div>
              <div className="mt-0.5 text-[11px] text-muted-foreground">
                {t(
                  "settings.knowledgePassiveRecall.showSnippetDesc",
                  "Include a one-line excerpt under each title (more tokens).",
                )}
              </div>
            </div>
            <Switch
              checked={draft.showSnippet}
              onCheckedChange={(v) => setDraft({ ...draft, showSnippet: v })}
            />
          </div>
          <div className="flex justify-end">
            <Button
              size="sm"
              disabled={!dirty || saving}
              onClick={() => void persist(draft, true)}
              className={cn(saveStatus === "failed" && "bg-destructive hover:bg-destructive/90")}
            >
              {saving ? (
                <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
              ) : saveStatus === "saved" ? (
                <Check className="mr-1.5 h-3.5 w-3.5 text-emerald-300" />
              ) : null}
              {t("common.save", "Save")}
            </Button>
          </div>
        </div>
      </AnimatedCollapse>
    </div>
  )
}

const MIB = 1024 * 1024

/**
 * Optional original-media retention for Knowledge Compiler sources. This is a
 * privacy/space setting: the text raw-source snapshot remains the durable truth
 * even when original audio/video/image files are not retained.
 */
function MediaRetentionSection() {
  const { t } = useTranslation()
  const [loaded, setLoaded] = useState<KnowledgeMediaRetentionConfig | null>(null)
  const [draft, setDraft] = useState<KnowledgeMediaRetentionConfig | null>(null)
  const [loadError, setLoadError] = useState<KnowledgePanelOperationErrorToast | null>(null)
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")

  const applyConfig = useCallback((c: KnowledgeMediaRetentionConfig) => {
    setLoaded(c)
    setDraft(c)
    setLoadError(null)
  }, [])

  const reload = useCallback(async () => {
    try {
      const c = await getTransport().call<KnowledgeMediaRetentionConfig>(
        "knowledge_media_retention_config_get_cmd",
      )
      applyConfig(c)
    } catch (e) {
      logger.error("settings", "MediaRetentionSection::reload", "Failed to load config", e)
      setLoadError(knowledgeMediaRetentionOperationErrorToast("load", t, e))
    }
  }, [applyConfig, t])

  useEffect(() => {
    let cancelled = false
    getTransport()
      .call<KnowledgeMediaRetentionConfig>("knowledge_media_retention_config_get_cmd")
      .then((c) => {
        if (cancelled) return
        applyConfig(c)
      })
      .catch((e) => {
        logger.error("settings", "MediaRetentionSection::load", "Failed to load config", e)
        if (!cancelled) setLoadError(knowledgeMediaRetentionOperationErrorToast("load", t, e))
      })
    return () => {
      cancelled = true
    }
  }, [applyConfig, t])

  const persist = useCallback(
    async (next: KnowledgeMediaRetentionConfig, viaButton: boolean) => {
      if (viaButton) setSaving(true)
      try {
        const saved = await getTransport().call<KnowledgeMediaRetentionConfig>(
          "knowledge_media_retention_config_set_cmd",
          { config: next },
        )
        setLoaded(saved)
        if (viaButton) {
          setDraft(saved)
          setSaving(false)
          setSaveStatus("saved")
          setTimeout(() => setSaveStatus("idle"), 2000)
          toast.success(t("settings.knowledgeMediaRetention.saved", "Saved"))
        } else {
          setDraft((d) => (d ? { ...d, enabled: saved.enabled } : saved))
        }
      } catch (e) {
        logger.error("settings", "MediaRetentionSection::save", "Failed to save config", e)
        const operation = viaButton ? "save" : "toggle"
        if (viaButton) {
          setSaveStatus("failed")
          setTimeout(() => setSaveStatus("idle"), 2000)
        } else {
          setDraft((d) => (d ? { ...d, enabled: !next.enabled } : d))
        }
        const failure = knowledgeMediaRetentionOperationErrorToast(operation, t, e)
        toast.error(
          failure.title,
          failure.description ? { description: failure.description } : undefined,
        )
      } finally {
        if (viaButton) setSaving(false)
      }
    },
    [t],
  )

  if (!draft || !loaded) {
    return (
      <div className="rounded-lg border border-border bg-card">
        <div className="px-4 py-3">
          <div className="text-sm font-medium">
            {t("settings.knowledgeMediaRetention.title", "Original media retention")}
          </div>
          <div className="mt-0.5 text-xs text-muted-foreground">
            {t(
              "settings.knowledgeMediaRetention.description",
              "Optionally keep imported audio, video, and image originals for source evidence review.",
            )}
          </div>
          {loadError && (
            <div className="mt-3 flex items-start gap-2 rounded-md border border-amber-500/25 bg-amber-500/10 px-3 py-2 text-xs text-amber-800 dark:text-amber-200">
              <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
              <div className="min-w-0 flex-1">
                <div className="font-medium">{loadError.title}</div>
                {loadError.description ? (
                  <div className="mt-0.5 whitespace-pre-wrap text-amber-800/80 dark:text-amber-100/80">
                    {loadError.description}
                  </div>
                ) : null}
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="mt-1 h-7 px-2 text-xs"
                  onClick={() => void reload()}
                >
                  <RotateCcw className="mr-1.5 h-3.5 w-3.5" />
                  {t("common.retry", "Retry")}
                </Button>
              </div>
            </div>
          )}
        </div>
      </div>
    )
  }

  const enabled = draft.enabled
  const totalMiB = Math.round(draft.maxTotalBytes / MIB)
  const sourceMiB = Math.round(draft.maxSourceBytes / MIB)
  const dirty =
    loaded.maxTotalBytes !== draft.maxTotalBytes ||
    loaded.maxSourceBytes !== draft.maxSourceBytes ||
    loaded.thumbnailMaxEdgePx !== draft.thumbnailMaxEdgePx ||
    loaded.pruneWhenOverQuota !== draft.pruneWhenOverQuota

  return (
    <div className="rounded-lg border border-border bg-card">
      <div className="flex items-center justify-between px-4 py-3">
        <div className="min-w-0">
          <div className="text-sm font-medium">
            {t("settings.knowledgeMediaRetention.title", "Original media retention")}
          </div>
          <div className="mt-0.5 text-xs text-muted-foreground">
            {t(
              "settings.knowledgeMediaRetention.description",
              "Optionally keep imported audio, video, and image originals for source evidence review.",
            )}
          </div>
        </div>
        <div className="flex items-center gap-2">
          <SettingsResetControl
            scope="knowledge"
            resetSection="media_retention"
            sectionLabel={t("settings.knowledgeMediaRetention.title", "Original media retention")}
            level="region"
            onReset={reload}
          />
          <Switch
            checked={enabled}
            onCheckedChange={(v) => {
              setDraft({ ...draft, enabled: v })
              void persist({ ...loaded, enabled: v }, false)
            }}
          />
        </div>
      </div>

      <AnimatedCollapse open={enabled}>
        <div className="space-y-3 border-t border-border px-4 py-3">
          <div className="grid gap-3 md:grid-cols-3">
            <label className="space-y-1">
              <span className="text-xs font-medium">
                {t("settings.knowledgeMediaRetention.maxTotal", "Total quota (MiB)")}
              </span>
              <DeferredNumberInput
                min={10}
                max={102400}
                value={totalMiB}
                onValueCommit={(value) =>
                  setDraft({ ...draft, maxTotalBytes: Math.max(10, value) * MIB })
                }
                className="h-8 text-xs"
              />
            </label>
            <label className="space-y-1">
              <span className="text-xs font-medium">
                {t("settings.knowledgeMediaRetention.maxSource", "Per source (MiB)")}
              </span>
              <DeferredNumberInput
                min={1}
                max={2048}
                value={sourceMiB}
                onValueCommit={(value) =>
                  setDraft({ ...draft, maxSourceBytes: Math.max(1, value) * MIB })
                }
                className="h-8 text-xs"
              />
            </label>
            <label className="space-y-1">
              <span className="text-xs font-medium">
                {t("settings.knowledgeMediaRetention.thumbnail", "Thumbnail edge (px)")}
              </span>
              <DeferredNumberInput
                min={128}
                max={2048}
                value={draft.thumbnailMaxEdgePx}
                onValueCommit={(value) => setDraft({ ...draft, thumbnailMaxEdgePx: value })}
                className="h-8 text-xs"
              />
            </label>
          </div>
          <div className="flex items-center justify-between">
            <div className="min-w-0 pr-3">
              <div className="text-xs font-medium">
                {t("settings.knowledgeMediaRetention.prune", "Prune oldest media over quota")}
              </div>
              <div className="mt-0.5 text-[11px] text-muted-foreground">
                {t(
                  "settings.knowledgeMediaRetention.pruneDesc",
                  "When a new retained file would exceed the quota, remove the oldest retained originals before skipping the new file.",
                )}
              </div>
            </div>
            <Switch
              checked={draft.pruneWhenOverQuota}
              onCheckedChange={(v) => setDraft({ ...draft, pruneWhenOverQuota: v })}
            />
          </div>
          <div className="flex justify-end">
            <Button
              size="sm"
              disabled={!dirty || saving}
              onClick={() => void persist(draft, true)}
              className={cn(saveStatus === "failed" && "bg-destructive hover:bg-destructive/90")}
            >
              {saving ? (
                <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
              ) : saveStatus === "saved" ? (
                <Check className="mr-1.5 h-3.5 w-3.5 text-emerald-300" />
              ) : null}
              {t("common.save", "Save")}
            </Button>
          </div>
        </div>
      </AnimatedCollapse>
    </div>
  )
}

/**
 * Collapsible advanced section for note chunking (D12). Two scalars — chunk size
 * and overlap (code points) — both clamped server-side. Saving triggers a full
 * reindex of every KB (re-chunk + re-embed), so it carries an explicit warning;
 * progress surfaces in the reembed card above + the Knowledge view task panel.
 * Applies to FTS even when vector search is off, so it's always shown.
 */
function ChunkAdvancedSection() {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [loaded, setLoaded] = useState<ChunkConfig | null>(null)
  const [maxChars, setMaxChars] = useState("")
  const [overlapChars, setOverlapChars] = useState("")
  const [loadError, setLoadError] = useState<KnowledgePanelOperationErrorToast | null>(null)
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")

  const applyConfig = useCallback((c: ChunkConfig) => {
    setLoaded(c)
    setMaxChars(String(c.maxChars))
    setOverlapChars(String(c.overlapChars))
    setLoadError(null)
  }, [])

  const reload = useCallback(async () => {
    try {
      const c = await getTransport().call<ChunkConfig>("knowledge_chunk_get_cmd")
      applyConfig(c)
    } catch (e) {
      logger.error("settings", "ChunkAdvancedSection::reload", "Failed to load chunk config", e)
      setLoadError(knowledgeChunkOperationErrorToast("load", t, e))
    }
  }, [applyConfig, t])

  useEffect(() => {
    let cancelled = false
    getTransport()
      .call<ChunkConfig>("knowledge_chunk_get_cmd")
      .then((c) => {
        if (cancelled) return
        applyConfig(c)
      })
      .catch((e) => {
        logger.error("settings", "ChunkAdvancedSection::load", "Failed to load chunk config", e)
        if (!cancelled) setLoadError(knowledgeChunkOperationErrorToast("load", t, e))
      })
    return () => {
      cancelled = true
    }
  }, [applyConfig, t])

  const mc = Number.parseInt(maxChars, 10)
  const oc = Number.parseInt(overlapChars, 10)
  const valid = Number.isFinite(mc) && Number.isFinite(oc) && mc > 0 && oc >= 0
  const dirty =
    !!loaded && (String(loaded.maxChars) !== maxChars || String(loaded.overlapChars) !== overlapChars)

  const save = useCallback(async () => {
    if (!valid || saving) return
    setSaving(true)
    try {
      const c = await getTransport().call<ChunkConfig>("knowledge_chunk_set_cmd", {
        maxChars: mc,
        overlapChars: oc,
      })
      applyConfig(c)
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
      toast.success(t("settings.knowledgeChunk.saved", "Saved — rebuilding every space"))
    } catch (e) {
      logger.error("settings", "ChunkAdvancedSection::save", "Failed to save chunk config", e)
      setSaveStatus("failed")
      setTimeout(() => setSaveStatus("idle"), 2000)
      const failure = knowledgeChunkOperationErrorToast("save", t, e)
      toast.error(
        failure.title,
        failure.description ? { description: failure.description } : undefined,
      )
    } finally {
      setSaving(false)
    }
  }, [valid, saving, mc, oc, applyConfig, t])

  return (
    <div className="rounded-lg border border-border bg-card">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center justify-between px-4 py-3"
      >
        <span className="flex items-center gap-2 text-sm font-medium">
          <Settings2 className="h-4 w-4 text-muted-foreground" />
          {t("settings.knowledgeChunk.title", "Advanced · chunking")}
        </span>
        <ChevronDown className={cn("h-4 w-4 transition-transform", open && "rotate-180")} />
      </button>
      <AnimatedCollapse open={open}>
        <div className="space-y-3 border-t border-border px-4 py-3">
          <p className="text-xs text-muted-foreground">
            {t(
              "settings.knowledgeChunk.description",
              "How notes are split into retrieval units. Defaults suit most notes.",
            )}
          </p>

          {loadError && (
            <div className="flex items-start gap-2 rounded-md border border-amber-500/25 bg-amber-500/10 px-3 py-2 text-xs text-amber-800 dark:text-amber-200">
              <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
              <div className="min-w-0 flex-1">
                <div className="font-medium">{loadError.title}</div>
                {loadError.description ? (
                  <div className="mt-0.5 whitespace-pre-wrap text-amber-800/80 dark:text-amber-100/80">
                    {loadError.description}
                  </div>
                ) : null}
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="mt-1 h-7 px-2 text-xs"
                  onClick={() => void reload()}
                >
                  <RotateCcw className="mr-1.5 h-3.5 w-3.5" />
                  {t("common.retry", "Retry")}
                </Button>
              </div>
            </div>
          )}

          {loaded && (
            <>
              <div className="grid grid-cols-2 gap-3">
                <label className="space-y-1">
                  <span className="text-xs font-medium">
                    {t("settings.knowledgeChunk.maxChars", "Chunk size (chars)")}
                  </span>
                  <NumberInput
                    min={200}
                    max={8000}
                    value={maxChars}
                    onChange={(e) => setMaxChars(e.target.value)}
                    className="h-8 text-xs"
                  />
                </label>
                <label className="space-y-1">
                  <span className="text-xs font-medium">
                    {t("settings.knowledgeChunk.overlap", "Overlap (chars)")}
                  </span>
                  <NumberInput
                    min={0}
                    value={overlapChars}
                    onChange={(e) => setOverlapChars(e.target.value)}
                    className="h-8 text-xs"
                  />
                </label>
              </div>
              <p className="flex items-start gap-1.5 text-[11px] text-amber-600 dark:text-amber-400">
                <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                {t(
                  "settings.knowledgeChunk.rebuildWarning",
                  "Saving rebuilds the index for every knowledge space.",
                )}
              </p>
              <div className="flex justify-end">
                <Button
                  size="sm"
                  disabled={!valid || !dirty || saving}
                  onClick={() => void save()}
                  className={cn(saveStatus === "failed" && "bg-destructive hover:bg-destructive/90")}
                >
                  {saving ? (
                    <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                  ) : saveStatus === "saved" ? (
                    <Check className="mr-1.5 h-3.5 w-3.5 text-emerald-300" />
                  ) : null}
                  {t("common.save", "Save")}
                </Button>
              </div>
            </>
          )}
        </div>
      </AnimatedCollapse>
    </div>
  )
}

/**
 * Hybrid `note_search` ranking tuning (`AppConfig.knowledge_search`). Pure
 * query-time (no reindex), so this is a normal saved setting. A one-click
 * "restore defaults" persists the best-practice values so a bad edit is never
 * sticky.
 */
/**
 * Per-field raw string draft. Holding the raw input string (not a parsed number)
 * keeps in-progress entry like "" or "0." intact instead of coercing through
 * `Number()` on every keystroke — which would turn "" into 0 (snapping min-1
 * fields to an invalid 0) and drop the trailing dot mid-decimal. Mirrors
 * `ChunkAdvancedSection`'s string-state pattern.
 */
type SearchDraft = Record<keyof KnowledgeSearchConfig, string>

const toSearchDraft = (c: KnowledgeSearchConfig): SearchDraft => ({
  textWeight: String(c.textWeight),
  vectorWeight: String(c.vectorWeight),
  rrfK: String(c.rrfK),
  mmrLambda: String(c.mmrLambda),
  candidateMultiplier: String(c.candidateMultiplier),
})

/**
 * Parse the string draft into a config, or `null` if any field isn't a finite
 * number. `candidateMultiplier` is a Rust `usize`, so it is parsed as an integer
 * — a decimal there would be rejected by serde and hard-fail the save.
 */
const parseSearchDraft = (d: SearchDraft): KnowledgeSearchConfig | null => {
  const cfg: KnowledgeSearchConfig = {
    textWeight: Number.parseFloat(d.textWeight),
    vectorWeight: Number.parseFloat(d.vectorWeight),
    rrfK: Number.parseFloat(d.rrfK),
    mmrLambda: Number.parseFloat(d.mmrLambda),
    candidateMultiplier: Number.parseInt(d.candidateMultiplier, 10),
  }
  return (Object.values(cfg) as number[]).every((n) => Number.isFinite(n)) ? cfg : null
}

function SearchRankingSection() {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [loaded, setLoaded] = useState<KnowledgeSearchConfig | null>(null)
  const [draft, setDraft] = useState<SearchDraft | null>(null)
  const [loadError, setLoadError] = useState<KnowledgePanelOperationErrorToast | null>(null)
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")

  const reload = useCallback(async () => {
    try {
      const c = await getTransport().call<KnowledgeSearchConfig>(
        "knowledge_search_config_get_cmd",
      )
      setLoaded(c)
      setDraft(toSearchDraft(c))
      setLoadError(null)
    } catch (e) {
      logger.error("settings", "SearchRankingSection::reload", "Failed to load search config", e)
      setLoadError(knowledgeSearchRankingOperationErrorToast("load", t, e))
    }
  }, [t])

  useEffect(() => {
    let cancelled = false
    getTransport()
      .call<KnowledgeSearchConfig>("knowledge_search_config_get_cmd")
      .then((c) => {
        if (cancelled) return
        setLoaded(c)
        setDraft(toSearchDraft(c))
        setLoadError(null)
      })
      .catch((e) => {
        logger.error("settings", "SearchRankingSection::load", "Failed to load search config", e)
        if (!cancelled) setLoadError(knowledgeSearchRankingOperationErrorToast("load", t, e))
      })
    return () => {
      cancelled = true
    }
  }, [t])

  const parsed = draft ? parseSearchDraft(draft) : null
  const dirty = !!loaded && !!parsed && JSON.stringify(loaded) !== JSON.stringify(parsed)

  const persist = useCallback(
    async (cfg: KnowledgeSearchConfig, operation: "save" | "restore") => {
      setSaving(true)
      try {
        const saved = await getTransport().call<KnowledgeSearchConfig>(
          "knowledge_search_config_set_cmd",
          { config: cfg },
        )
        setLoaded(saved)
        setDraft(toSearchDraft(saved))
        setSaveStatus("saved")
        setTimeout(() => setSaveStatus("idle"), 2000)
      } catch (e) {
        logger.error("settings", "SearchRankingSection::save", "Failed to save search config", e)
        setSaveStatus("failed")
        setTimeout(() => setSaveStatus("idle"), 2000)
        const failure = knowledgeSearchRankingOperationErrorToast(operation, t, e)
        toast.error(
          failure.title,
          failure.description ? { description: failure.description } : undefined,
        )
      } finally {
        setSaving(false)
      }
    },
    [t],
  )

  const num = (
    key: keyof KnowledgeSearchConfig,
    opts: { min: number; max: number; step?: number },
  ) =>
    draft && (
      <NumberInput
        min={opts.min}
        max={opts.max}
        step={opts.step}
        value={draft[key]}
        onChange={(e) => setDraft((d) => (d ? { ...d, [key]: e.target.value } : d))}
        className="h-8 w-20 text-xs"
      />
    )

  return (
    <div className="rounded-lg border border-border bg-card">
      <div className="flex items-center gap-2 pr-4">
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          className="flex min-w-0 flex-1 items-center justify-between px-4 py-3"
        >
          <span className="flex items-center gap-2 text-sm font-medium">
            <Settings2 className="h-4 w-4 text-muted-foreground" />
            {t("settings.knowledgeSearch.title", "Advanced · search ranking")}
          </span>
          <ChevronDown className={cn("h-4 w-4 transition-transform", open && "rotate-180")} />
        </button>
        <SettingsResetControl
          scope="knowledge"
          resetSection="search"
          sectionLabel={t("settings.knowledgeSearch.title", "Advanced · search ranking")}
          level="region"
          onReset={reload}
        />
      </div>
      <AnimatedCollapse open={open}>
        <div className="space-y-3 border-t border-border px-4 py-3">
          <p className="text-xs text-muted-foreground">
            {t(
              "settings.knowledgeSearch.description",
              "How note_search ranks results: it runs keyword (BM25) and semantic (vector) search over note chunks, fuses the two rankings (RRF), then re-ranks for diversity (MMR). Defaults suit most libraries — only touch these if results feel off. Saving takes effect immediately, with no reindex.",
            )}
          </p>

          {loadError && (
            <div className="flex items-start gap-2 rounded-md border border-amber-500/25 bg-amber-500/10 px-3 py-2 text-xs text-amber-800 dark:text-amber-200">
              <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
              <div className="min-w-0 flex-1">
                <div className="font-medium">{loadError.title}</div>
                {loadError.description ? (
                  <div className="mt-0.5 whitespace-pre-wrap text-amber-800/80 dark:text-amber-100/80">
                    {loadError.description}
                  </div>
                ) : null}
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="mt-1 h-7 px-2 text-xs"
                  onClick={() => void reload()}
                >
                  <RotateCcw className="mr-1.5 h-3.5 w-3.5" />
                  {t("common.retry", "Retry")}
                </Button>
              </div>
            </div>
          )}

          {draft && (
            <div className="space-y-3">
              <Field
                label={t("settings.knowledgeSearch.textWeight", "Keyword weight")}
                desc={t(
                  "settings.knowledgeSearch.textWeightDesc",
                  "How much exact word/phrase matches count in fusion. Raise it for code, names, or jargon where the literal term matters; only the ratio to the semantic weight matters.",
                )}
              >
                {num("textWeight", { min: 0, max: 1, step: 0.05 })}
              </Field>
              <Field
                label={t("settings.knowledgeSearch.vectorWeight", "Semantic weight")}
                desc={t(
                  "settings.knowledgeSearch.vectorWeightDesc",
                  "How much meaning-based (vector) similarity counts. Raise it to surface related notes that use different words. Has no effect if the vector model is off (keyword-only).",
                )}
              >
                {num("vectorWeight", { min: 0, max: 1, step: 0.05 })}
              </Field>
              <Field
                label={t("settings.knowledgeSearch.rrfK", "Fusion smoothing (RRF k)")}
                desc={t(
                  "settings.knowledgeSearch.rrfKDesc",
                  "How sharply the top of each ranking dominates. Lower = trust each method's #1 hit more; higher = blend more evenly across the top results. 60 is the standard balance.",
                )}
              >
                {num("rrfK", { min: 1, max: 1000, step: 1 })}
              </Field>
              <Field
                label={t("settings.knowledgeSearch.mmrLambda", "Diversity (MMR λ)")}
                desc={t(
                  "settings.knowledgeSearch.mmrLambdaDesc",
                  "Relevance vs. variety. 1.0 = pure relevance (may return several near-identical notes); lower mixes in more varied results. 0.7 keeps results relevant while trimming duplicates.",
                )}
              >
                {num("mmrLambda", { min: 0, max: 1, step: 0.05 })}
              </Field>
              <Field
                label={t("settings.knowledgeSearch.candidateMultiplier", "Candidate pool (×limit)")}
                desc={t(
                  "settings.knowledgeSearch.candidateMultiplierDesc",
                  "How many candidates to gather before diversity re-ranking, as a multiple of the requested result count. Higher = better diversity/recall at a small cost; lower = faster.",
                )}
              >
                {num("candidateMultiplier", { min: 1, max: 10, step: 1 })}
              </Field>
            </div>
          )}

          <div className="flex items-center justify-end border-t border-border pt-3">
            <Button
              size="sm"
              disabled={!dirty || !parsed || saving}
              onClick={() => parsed && void persist(parsed, "save")}
              className={cn(saveStatus === "failed" && "bg-destructive hover:bg-destructive/90")}
            >
              {saving ? (
                <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
              ) : saveStatus === "saved" ? (
                <Check className="mr-1.5 h-3.5 w-3.5 text-emerald-300" />
              ) : null}
              {t("common.save", "Save")}
            </Button>
          </div>
        </div>
      </AnimatedCollapse>
    </div>
  )
}

/** Label + description + control row for the search-ranking fields. */
function Field({
  label,
  desc,
  children,
}: {
  label: string
  desc: string
  children: React.ReactNode
}) {
  return (
    <div className="flex items-start justify-between gap-3">
      <div className="min-w-0">
        <div className="text-xs font-medium">{label}</div>
        <div className="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">{desc}</div>
      </div>
      <div className="shrink-0 pt-0.5">{children}</div>
    </div>
  )
}
