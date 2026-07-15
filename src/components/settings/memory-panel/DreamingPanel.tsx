import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { AlertTriangle, Check, Loader2 } from "lucide-react"

import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { DeferredNumberInput } from "@/components/ui/deferred-number-input"
import { Switch } from "@/components/ui/switch"
import type { AvailableModel } from "@/components/ui/model-selector"
import { ModelChainEditor, type ModelChainRef } from "@/components/ui/model-chain-editor"
import CronExpressionBuilder from "@/components/cron/CronExpressionBuilder"
import { buildCronFromVisual, parseCronToVisual } from "@/components/cron/cronHelpers"
import type { CronFrequency } from "@/components/cron/CronJobForm.types"
import {
  dreamingSettingsOperationErrorToast,
  type DreamingSettingsOperationErrorToast,
} from "./dreamingSettingsOperationFeedback"

interface IdleTriggerConfig {
  enabled: boolean
  idleMinutes: number
}
interface CronTriggerConfig {
  enabled: boolean
  cronExpr: string
}
interface PromotionThresholds {
  minScore: number
  maxPromote: number
}
interface ProfileSynthesisConfig {
  enabled: boolean
  maxLinesPerScope: number
}
interface DeepResolverConfig {
  autoExpireOnLightCycle: boolean
  autoResolveOnLightCycle: boolean
  autoResolveMaxGroups: number
  autoResolveMinConfidence: number
  autoMergeNearDuplicates: boolean
  autoMergeSimilarity: number
}
interface DreamingConfig {
  enabled: boolean
  idleTrigger: IdleTriggerConfig
  cronTrigger: CronTriggerConfig
  manualEnabled: boolean
  promotion: PromotionThresholds
  scopeDays: number
  candidateLimit: number
  narrativeMaxTokens: number
  narrativeTimeoutSecs: number
  /** Deprecated — superseded by `modelOverride`. Read-only display concern. */
  narrativeModel?: string | null
  modelOverride?: ModelChainRef | null
  profileSynthesis: ProfileSynthesisConfig
  deepResolver?: DeepResolverConfig
}

interface DreamReport {
  trigger: "idle" | "cron" | "manual"
  candidatesScanned: number
  candidatesNominated: number
  promoted: { id: number }[]
  diaryPath: string | null
  durationMs: number
  note: string | null
}

interface IdleStatus {
  lastActivityEpochSecs: number
  idleMinutes: number
}

type SaveStatus = "idle" | "saved" | "failed"

export default function DreamingPanel() {
  const { t } = useTranslation()
  const [cfg, setCfg] = useState<DreamingConfig | null>(null)
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<SaveStatus>("idle")
  const [loadError, setLoadError] = useState<DreamingSettingsOperationErrorToast | null>(null)
  const [saveError, setSaveError] = useState<DreamingSettingsOperationErrorToast | null>(null)
  const [saveRecoveryError, setSaveRecoveryError] =
    useState<DreamingSettingsOperationErrorToast | null>(null)
  const [modelLoadError, setModelLoadError] =
    useState<DreamingSettingsOperationErrorToast | null>(null)
  const [statusLoadError, setStatusLoadError] =
    useState<DreamingSettingsOperationErrorToast | null>(null)
  const [availableModels, setAvailableModels] = useState<AvailableModel[]>([])
  const [lastReport, setLastReport] = useState<DreamReport | null>(null)
  const [idleStatus, setIdleStatus] = useState<IdleStatus | null>(null)
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000))
  const [cronInvalid, setCronInvalid] = useState(false)

  // ── Initial load + live config sync via `config:changed` ──
  const loadCfg = useCallback(async () => {
    setLoadError(null)
    try {
      const c = await getTransport().call<DreamingConfig>("get_dreaming_config")
      setCfg(c)
      setLoadError(null)
    } catch (e) {
      logger.error("settings", "DreamingPanel::load", "Failed to load config", e)
      setLoadError(dreamingSettingsOperationErrorToast("loadConfig", t, e))
    }
  }, [t])

  const loadModels = useCallback(async () => {
    setModelLoadError(null)
    try {
      const models = await getTransport().call<AvailableModel[]>("get_available_models")
      setAvailableModels(models ?? [])
      setModelLoadError(null)
    } catch (e) {
      logger.warn("settings", "DreamingPanel::models", "Failed to load models", e)
      setAvailableModels([])
      setModelLoadError(dreamingSettingsOperationErrorToast("loadModels", t, e))
    }
  }, [t])

  const loadStatus = useCallback(async () => {
    let failed = false
    const [report, idle] = await Promise.all([
      getTransport()
        .call<DreamReport | null>("dreaming_last_report")
        .catch((e) => {
          failed = true
          logger.warn("settings", "DreamingPanel::lastReport", "Failed to load last report", e)
          setStatusLoadError((current) =>
            current ?? dreamingSettingsOperationErrorToast("loadStatus", t, e),
          )
          return null
        }),
      getTransport()
        .call<IdleStatus>("dreaming_idle_status")
        .catch((e) => {
          failed = true
          logger.warn("settings", "DreamingPanel::idleStatus", "Failed to load idle status", e)
          setStatusLoadError((current) =>
            current ?? dreamingSettingsOperationErrorToast("loadStatus", t, e),
          )
          return null
        }),
    ])
    setLastReport(report ?? null)
    setIdleStatus(idle ?? null)
    if (!failed) setStatusLoadError(null)
  }, [t])

  useEffect(() => {
    let cancelled = false
    setLoadError(null)
    Promise.all([
      getTransport().call<DreamingConfig>("get_dreaming_config"),
      loadModels(),
      loadStatus(),
    ])
      .then(([c]) => {
        if (cancelled) return
        setCfg(c)
        setLoadError(null)
        setLoading(false)
      })
      .catch((e: unknown) => {
        if (cancelled) return
        logger.error("settings", "DreamingPanel::loadAll", "Initial load failed", e)
        setLoadError(dreamingSettingsOperationErrorToast("loadConfig", t, e))
        setLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [loadModels, loadStatus, t])

  useEffect(() => {
    return getTransport().listen("config:changed", (raw) => {
      const payload = raw as { category?: string }
      if (payload.category === "dreaming") {
        void loadCfg()
      }
    })
  }, [loadCfg])

  useEffect(() => {
    return getTransport().listen("dreaming:cycle_complete", () => {
      void loadStatus()
    })
  }, [loadStatus])

  // 1Hz tick to drive the idle-countdown re-render. Only mounted while
  // the countdown is actually visible — keeps the panel idle when the
  // idle trigger is off or the activity timestamp is unknown.
  const countdownActive =
    !!cfg?.enabled &&
    !!cfg?.idleTrigger.enabled &&
    !!idleStatus &&
    idleStatus.lastActivityEpochSecs > 0
  useEffect(() => {
    if (!countdownActive) return
    const id = setInterval(() => setNow(Math.floor(Date.now() / 1000)), 1000)
    return () => clearInterval(id)
  }, [countdownActive])

  // ── Debounced save (mirrors AwarenessPanel) ──
  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const save = useCallback((next: DreamingConfig) => {
    setCfg(next)
    setSaveError(null)
    setSaveRecoveryError(null)
    if (saveTimer.current) clearTimeout(saveTimer.current)
    saveTimer.current = setTimeout(async () => {
      setSaving(true)
      try {
        await getTransport().call("save_dreaming_config", { config: next })
        setSaveError(null)
        setSaveRecoveryError(null)
        setSaveStatus("saved")
        setTimeout(() => setSaveStatus("idle"), 1500)
      } catch (e) {
        logger.error("settings", "DreamingPanel::save", "Failed to save", e)
        setSaveError(dreamingSettingsOperationErrorToast("saveConfig", t, e))
        setSaveStatus("failed")
        setTimeout(() => setSaveStatus("idle"), 1500)
        try {
          const fresh = await getTransport().call<DreamingConfig>("get_dreaming_config")
          setCfg(fresh)
          setSaveRecoveryError(null)
        } catch (reloadError) {
          logger.warn(
            "settings",
            "DreamingPanel::reloadAfterSaveFailure",
            "Failed to reload config after save failure",
            reloadError,
          )
          setSaveRecoveryError(
            dreamingSettingsOperationErrorToast("reloadAfterSave", t, reloadError),
          )
        }
      } finally {
        setSaving(false)
      }
    }, 500)
  }, [t])

  // ── CronExpressionBuilder state ──
  const [cronFreq, setCronFreq] = useState<CronFrequency>("daily")
  const [cronHour, setCronHour] = useState("03")
  const [cronMinute, setCronMinute] = useState("00")
  const [cronWeekdays, setCronWeekdays] = useState<boolean[]>(Array(7).fill(false))
  const [cronMonthDay, setCronMonthDay] = useState("1")
  const [cronRawExpr, setCronRawExpr] = useState("0 0 3 * * *")
  // Last expression we either hydrated from or saved to. Distinguishes
  // "user just edited visual fields" (save) from "config arrived externally"
  // (re-hydrate visual). Without it the visual→raw→save loop double-saves
  // on initial mount and ignores `config:changed` from the ha-settings skill.
  const lastSyncedExpr = useRef<string | null>(null)

  // Hydrate visual fields whenever cron_expr changes externally
  // (initial load or `config:changed` from skill).
  useEffect(() => {
    if (loading || !cfg) return
    const incoming = cfg.cronTrigger.cronExpr || "0 0 3 * * *"
    if (incoming === lastSyncedExpr.current) return
    const v = parseCronToVisual(incoming)
    setCronFreq(v.freq)
    setCronHour(v.hour)
    setCronMinute(v.minute)
    setCronWeekdays(v.weekdays)
    setCronMonthDay(v.monthDay)
    setCronRawExpr(incoming)
    lastSyncedExpr.current = incoming
  }, [loading, cfg])

  const cronExpression = useMemo(
    () =>
      buildCronFromVisual(
        cronFreq,
        cronHour,
        cronMinute,
        cronWeekdays,
        cronMonthDay,
        cronRawExpr,
      ),
    [cronFreq, cronHour, cronMinute, cronWeekdays, cronMonthDay, cronRawExpr],
  )

  // Persist visual edits, skipping the echo from initial hydration.
  useEffect(() => {
    if (loading || !cfg) return
    if (cronExpression === lastSyncedExpr.current) return
    lastSyncedExpr.current = cronExpression
    save({ ...cfg, cronTrigger: { ...cfg.cronTrigger, cronExpr: cronExpression } })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cronExpression])

  // Validate cron expression (non-blocking warning).
  useEffect(() => {
    let cancelled = false
    if (!cronExpression) {
      setCronInvalid(false)
      return
    }
    void getTransport()
      .call<void>("validate_cron_expression", { expression: cronExpression })
      .then(() => {
        if (!cancelled) setCronInvalid(false)
      })
      .catch(() => {
        if (!cancelled) setCronInvalid(true)
      })
    return () => {
      cancelled = true
    }
  }, [cronExpression])

  const toggleWeekday = (idx: number) => {
    const next = [...cronWeekdays]
    next[idx] = !next[idx]
    setCronWeekdays(next)
  }

  if (loading) return null

  if (!cfg) {
    return (
      <div className="flex-1 overflow-y-auto p-6">
        <div className="rounded-lg border border-amber-500/30 bg-amber-500/5 p-4 text-sm">
          <div className="font-medium text-foreground">
            {loadError?.title ??
              t("settings.dreaming.errors.loadConfig", {
                defaultValue: "Failed to load Dreaming settings",
              })}
          </div>
          {loadError?.description && (
            <div className="mt-1 break-all text-xs text-muted-foreground">
              {loadError.description}
            </div>
          )}
          <button
            type="button"
            className="mt-3 text-xs font-medium text-primary underline"
            onClick={() => {
              setLoading(true)
              void loadCfg().finally(() => setLoading(false))
            }}
          >
            {t("common.refresh")}
          </button>
        </div>
      </div>
    )
  }

  const masterDisabled = !cfg.enabled
  const deepResolver: DeepResolverConfig = {
    autoExpireOnLightCycle: true,
    autoResolveOnLightCycle: true,
    autoResolveMaxGroups: 8,
    autoResolveMinConfidence: 0.92,
    autoMergeNearDuplicates: true,
    autoMergeSimilarity: 0.84,
    ...(cfg.deepResolver ?? {}),
  }

  // Status row idle countdown.
  const idleCountdownSecs =
    cfg.enabled && cfg.idleTrigger.enabled && idleStatus && idleStatus.lastActivityEpochSecs > 0
      ? Math.max(0, idleStatus.lastActivityEpochSecs + idleStatus.idleMinutes * 60 - now)
      : null

  return (
    <div className="flex-1 overflow-y-auto p-6 space-y-6">
      {/* ── Status row ── */}
      {cfg.enabled && (
        <div className="rounded-lg border bg-secondary/30 p-3 text-xs space-y-1">
          {statusLoadError && (
            <div className="rounded border border-amber-500/30 bg-amber-500/5 px-2 py-1.5">
              <div className="font-medium text-foreground">{statusLoadError.title}</div>
              {statusLoadError.description && (
                <div className="mt-1 break-all text-muted-foreground">
                  {statusLoadError.description}
                </div>
              )}
            </div>
          )}
          {lastReport ? (
            <div className="flex items-center justify-between gap-2">
              <div className="text-muted-foreground">
                {t("settings.dreaming.lastCycle")}{" "}
                <span className="text-foreground">
                  ({t(`dashboard.dreaming.trigger.${lastReport.trigger}`)})
                </span>
                {" · "}
                {t("settings.dreaming.scanned", { count: lastReport.candidatesScanned })}
                {" · "}
                {t("settings.dreaming.nominated", { count: lastReport.candidatesNominated })}
                {" · "}
                {t("settings.dreaming.promoted", { count: lastReport.promoted.length })}
                {" · "}
                {(lastReport.durationMs / 1000).toFixed(1)}s
              </div>
            </div>
          ) : (
            <div className="text-muted-foreground italic">
              {t("settings.dreaming.noCycleYet")}
            </div>
          )}
          {idleCountdownSecs != null && (
            <div className="text-muted-foreground">
              {t("settings.dreaming.idleCountdown", {
                minutes: Math.ceil(idleCountdownSecs / 60),
              })}
            </div>
          )}
        </div>
      )}

      {/* ── Header (title + master switch) ── */}
      <div className="flex items-center justify-between">
        <div>
          <div className="text-sm font-medium">{t("settings.dreaming.title")}</div>
          <div className="text-xs text-muted-foreground mt-0.5">
            {t("settings.dreaming.desc")}
          </div>
        </div>
        <div className="flex items-center gap-2">
          {saving && <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />}
          {saveStatus === "saved" && <Check className="h-4 w-4 text-emerald-500" />}
          <Switch
            checked={cfg.enabled}
            onCheckedChange={(v) => save({ ...cfg, enabled: v })}
            aria-label={t("settings.dreaming.title")}
          />
        </div>
      </div>

      {saveError && (
        <div className="rounded-lg border border-amber-500/30 bg-amber-500/5 p-3 text-xs">
          <div className="font-medium text-foreground">{saveError.title}</div>
          {saveError.description && (
            <div className="mt-1 break-all text-muted-foreground">{saveError.description}</div>
          )}
        </div>
      )}
      {saveRecoveryError && (
        <div className="rounded-lg border border-amber-500/30 bg-amber-500/5 p-3 text-xs">
          <div className="font-medium text-foreground">{saveRecoveryError.title}</div>
          {saveRecoveryError.description && (
            <div className="mt-1 break-all text-muted-foreground">
              {saveRecoveryError.description}
            </div>
          )}
        </div>
      )}

      <div className={masterDisabled ? "pointer-events-none opacity-50" : ""}>
        {/* ── Idle trigger ── */}
        <Section title={t("settings.dreaming.idleTriggerTitle")}>
          <Row
            label={t("settings.dreaming.idleTriggerEnabled")}
            desc={t("settings.dreaming.idleTriggerEnabledDesc")}
            control={
              <Switch
                checked={cfg.idleTrigger.enabled}
                aria-label={t("settings.dreaming.idleTriggerEnabled")}
                onCheckedChange={(v) =>
                  save({ ...cfg, idleTrigger: { ...cfg.idleTrigger, enabled: v } })
                }
              />
            }
          />
          <NumberRow
            label={t("settings.dreaming.idleMinutes")}
            desc={t("settings.dreaming.idleMinutesDesc")}
            min={5}
            step={5}
            value={cfg.idleTrigger.idleMinutes}
            onChange={(v) => save({ ...cfg, idleTrigger: { ...cfg.idleTrigger, idleMinutes: v } })}
          />
        </Section>

        {/* ── Cron trigger ── */}
        <Section title={t("settings.dreaming.cronTriggerTitle")}>
          <Row
            label={t("settings.dreaming.cronTriggerEnabled")}
            desc={t("settings.dreaming.cronTriggerEnabledDesc")}
            control={
              <Switch
                checked={cfg.cronTrigger.enabled}
                aria-label={t("settings.dreaming.cronTriggerEnabled")}
                onCheckedChange={(v) =>
                  save({ ...cfg, cronTrigger: { ...cfg.cronTrigger, enabled: v } })
                }
              />
            }
          />
          <div className={cfg.cronTrigger.enabled ? "" : "pointer-events-none opacity-50"}>
            <CronExpressionBuilder
              cronFreq={cronFreq}
              setCronFreq={setCronFreq}
              cronHour={cronHour}
              setCronHour={setCronHour}
              cronMinute={cronMinute}
              setCronMinute={setCronMinute}
              cronWeekdays={cronWeekdays}
              toggleWeekday={toggleWeekday}
              cronMonthDay={cronMonthDay}
              setCronMonthDay={setCronMonthDay}
              cronRawExpr={cronRawExpr}
              setCronRawExpr={setCronRawExpr}
              cronExpression={cronExpression}
            />
            {cronInvalid && (
              <div className="mt-2 flex items-start gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 p-2 text-xs">
                <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-500" />
                <div>{t("settings.dreaming.cronExprInvalid")}</div>
              </div>
            )}
          </div>
        </Section>

        {/* ── Manual ── */}
        <Section title={t("settings.dreaming.manualTitle")}>
          <Row
            label={t("settings.dreaming.manualEnabled")}
            desc={t("settings.dreaming.manualEnabledDesc")}
            control={
              <Switch
                checked={cfg.manualEnabled}
                onCheckedChange={(v) => save({ ...cfg, manualEnabled: v })}
                aria-label={t("settings.dreaming.manualEnabled")}
              />
            }
          />
        </Section>

        {/* ── Promotion ── */}
        <Section title={t("settings.dreaming.promotionTitle")}>
          <NumberRow
            label={t("settings.dreaming.minScore")}
            desc={t("settings.dreaming.minScoreDesc")}
            min={0}
            max={1}
            step={0.05}
            value={cfg.promotion.minScore}
            onChange={(v) => save({ ...cfg, promotion: { ...cfg.promotion, minScore: v } })}
          />
          <NumberRow
            label={t("settings.dreaming.maxPromote")}
            desc={t("settings.dreaming.maxPromoteDesc")}
            min={1}
            value={cfg.promotion.maxPromote}
            onChange={(v) => save({ ...cfg, promotion: { ...cfg.promotion, maxPromote: v } })}
          />
        </Section>

        {/* ── Window ── */}
        <Section title={t("settings.dreaming.windowTitle")}>
          <NumberRow
            label={t("settings.dreaming.scopeDays")}
            desc={t("settings.dreaming.scopeDaysDesc")}
            min={1}
            value={cfg.scopeDays}
            onChange={(v) => save({ ...cfg, scopeDays: v })}
          />
          <NumberRow
            label={t("settings.dreaming.candidateLimit")}
            desc={t("settings.dreaming.candidateLimitDesc")}
            min={1}
            step={10}
            value={cfg.candidateLimit}
            onChange={(v) => save({ ...cfg, candidateLimit: v })}
          />
        </Section>

        {/* ── Narrative ── */}
        <Section title={t("settings.dreaming.narrativeTitle")}>
          <NumberRow
            label={t("settings.dreaming.narrativeMaxTokens")}
            desc={t("settings.dreaming.narrativeMaxTokensDesc")}
            min={256}
            step={256}
            value={cfg.narrativeMaxTokens}
            onChange={(v) => save({ ...cfg, narrativeMaxTokens: v })}
          />
          <NumberRow
            label={t("settings.dreaming.narrativeTimeoutSecs")}
            desc={t("settings.dreaming.narrativeTimeoutSecsDesc")}
            min={5}
            step={5}
            value={cfg.narrativeTimeoutSecs}
            onChange={(v) => save({ ...cfg, narrativeTimeoutSecs: v })}
          />
          <div className="space-y-1">
            <div className="text-sm font-medium">{t("settings.dreaming.narrativeModel")}</div>
            <div className="text-xs text-muted-foreground">
              {t("settings.dreaming.narrativeModelDesc")}
            </div>
            {modelLoadError && (
              <div className="rounded border border-amber-500/30 bg-amber-500/5 px-2 py-1.5 text-xs">
                <div className="font-medium text-foreground">{modelLoadError.title}</div>
                {modelLoadError.description && (
                  <div className="mt-1 break-all text-muted-foreground">
                    {modelLoadError.description}
                  </div>
                )}
              </div>
            )}
            <ModelChainEditor
              value={cfg.modelOverride ?? null}
              onChange={(next) => save({ ...cfg, modelOverride: next })}
              availableModels={availableModels}
              inheritLabel={t("settings.dreaming.narrativeModelDefault")}
            />
          </div>
        </Section>

        {/* ── Profile Synthesis (Phase 4) ── */}
        <Section title={t("settings.dreaming.profileTitle")}>
          <Row
            label={t("settings.dreaming.profileEnabled")}
            desc={t("settings.dreaming.profileEnabledDesc")}
            control={
              <Switch
                checked={cfg.profileSynthesis.enabled}
                aria-label={t("settings.dreaming.profileEnabled")}
                onCheckedChange={(v) =>
                  save({
                    ...cfg,
                    profileSynthesis: { ...cfg.profileSynthesis, enabled: v },
                  })
                }
              />
            }
          />
          <NumberRow
            label={t("settings.dreaming.profileMaxLines")}
            desc={t("settings.dreaming.profileMaxLinesDesc")}
            min={1}
            max={100}
            value={cfg.profileSynthesis.maxLinesPerScope}
            onChange={(v) =>
              save({
                ...cfg,
                profileSynthesis: { ...cfg.profileSynthesis, maxLinesPerScope: v },
              })
            }
          />
        </Section>

        {/* ── Deep Resolver ── */}
        <Section title={t("settings.dreaming.deepResolverTitle", "Deep Resolver")}>
          <Row
            label={t(
              "settings.dreaming.autoExpireOnLightCycle",
              "Auto-expire outdated claims"
            )}
            desc={t(
              "settings.dreaming.autoExpireOnLightCycleDesc",
              "During normal Dreaming cycles, persist deterministic valid-until expiry as an audit-backed resolver action."
            )}
            control={
              <Switch
                checked={deepResolver.autoExpireOnLightCycle}
                aria-label={t(
                  "settings.dreaming.autoExpireOnLightCycle",
                  "Auto-expire outdated claims",
                )}
                onCheckedChange={(v) =>
                  save({
                    ...cfg,
                    deepResolver: {
                      ...deepResolver,
                      autoExpireOnLightCycle: v,
                    },
                  })
                }
              />
            }
          />
          <Row
            label={t(
              "settings.dreaming.autoResolveOnLightCycle",
              "Automatically review memory conflicts",
            )}
            desc={t(
              "settings.dreaming.autoResolveOnLightCycleDesc",
              "Graph rules skip multi-value facts; a bounded LLM pass sends only high-confidence conflicts to Review Inbox and never auto-supersedes a fact.",
            )}
            control={
              <Switch
                checked={deepResolver.autoResolveOnLightCycle}
                aria-label={t(
                  "settings.dreaming.autoResolveOnLightCycle",
                  "Automatically review memory conflicts",
                )}
                onCheckedChange={(v) =>
                  save({
                    ...cfg,
                    deepResolver: {
                      ...deepResolver,
                      autoResolveOnLightCycle: v,
                    },
                  })
                }
              />
            }
          />
          {deepResolver.autoResolveOnLightCycle && (
            <>
              <NumberRow
                label={t("settings.dreaming.autoResolveMaxGroups", "Groups per cycle")}
                desc={t(
                  "settings.dreaming.autoResolveMaxGroupsDesc",
                  "Bounds background LLM calls and leaves overflow for the next cycle.",
                )}
                min={1}
                max={20}
                value={deepResolver.autoResolveMaxGroups}
                onChange={(v) =>
                  save({
                    ...cfg,
                    deepResolver: { ...deepResolver, autoResolveMaxGroups: v },
                  })
                }
              />
              <NumberRow
                label={t(
                  "settings.dreaming.autoResolveMinConfidence",
                  "Minimum LLM confidence",
                )}
                desc={t(
                  "settings.dreaming.autoResolveMinConfidenceDesc",
                  "Lower-confidence classifications leave claims untouched.",
                )}
                min={0.75}
                max={0.99}
                step={0.01}
                value={deepResolver.autoResolveMinConfidence}
                onChange={(v) =>
                  save({
                    ...cfg,
                    deepResolver: { ...deepResolver, autoResolveMinConfidence: v },
                  })
                }
              />
              <Row
                label={t(
                  "settings.dreaming.autoMergeNearDuplicates",
                  "Merge corroborated near-duplicates",
                )}
                desc={t(
                  "settings.dreaming.autoMergeNearDuplicatesDesc",
                  "Requires both a high-confidence duplicate verdict and graph alias or strong lexical similarity.",
                )}
                control={
                  <Switch
                    checked={deepResolver.autoMergeNearDuplicates}
                    aria-label={t(
                      "settings.dreaming.autoMergeNearDuplicates",
                      "Merge corroborated near-duplicates",
                    )}
                    onCheckedChange={(v) =>
                      save({
                        ...cfg,
                        deepResolver: {
                          ...deepResolver,
                          autoMergeNearDuplicates: v,
                        },
                      })
                    }
                  />
                }
              />
              {deepResolver.autoMergeNearDuplicates && (
                <NumberRow
                  label={t(
                    "settings.dreaming.autoMergeSimilarity",
                    "Near-duplicate similarity",
                  )}
                  desc={t(
                    "settings.dreaming.autoMergeSimilarityDesc",
                    "Minimum lexical corroboration when no graph alias connects the objects.",
                  )}
                  min={0.7}
                  max={0.98}
                  step={0.01}
                  value={deepResolver.autoMergeSimilarity}
                  onChange={(v) =>
                    save({
                      ...cfg,
                      deepResolver: { ...deepResolver, autoMergeSimilarity: v },
                    })
                  }
                />
              )}
            </>
          )}
        </Section>
      </div>
    </div>
  )
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="space-y-3">
      <div className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
        {title}
      </div>
      <div className="space-y-2">{children}</div>
    </div>
  )
}

function NumberRow({
  label,
  desc,
  min,
  max,
  step = 1,
  value,
  onChange,
}: {
  label: string
  desc?: string
  min: number
  max?: number
  step?: number
  value: number
  onChange: (v: number) => void
}) {
  const isInteger = Number.isInteger(step)
  return (
    <Row
      label={label}
      desc={desc}
      control={
        <DeferredNumberInput
          min={min}
          max={max}
          step={step}
          value={value}
          integer={isInteger}
          onValueCommit={onChange}
          className="w-24 h-8 text-sm text-right"
        />
      }
    />
  )
}

function Row({
  label,
  desc,
  control,
}: {
  label: string
  desc?: string
  control: React.ReactNode
}) {
  return (
    <div className="flex items-center justify-between gap-3 px-3 py-2.5 rounded-lg hover:bg-secondary/40 transition-colors">
      <div className="space-y-0.5 pr-4 min-w-0">
        <div className="text-sm font-medium">{label}</div>
        {desc && <div className="text-xs text-muted-foreground">{desc}</div>}
      </div>
      <div className="shrink-0">{control}</div>
    </div>
  )
}
