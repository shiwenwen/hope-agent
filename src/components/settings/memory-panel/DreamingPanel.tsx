import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { AlertTriangle, Check, Loader2 } from "lucide-react"

import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { ModelSelector, type AvailableModel } from "@/components/ui/model-selector"
import CronExpressionBuilder from "@/components/cron/CronExpressionBuilder"
import { buildCronFromVisual, parseCronToVisual } from "@/components/cron/cronHelpers"
import type { CronFrequency } from "@/components/cron/CronJobForm.types"

const NARRATIVE_MODEL_DEFAULT = "__default__"

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
  narrativeModel?: string | null
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
  const [availableModels, setAvailableModels] = useState<AvailableModel[]>([])
  const [lastReport, setLastReport] = useState<DreamReport | null>(null)
  const [idleStatus, setIdleStatus] = useState<IdleStatus | null>(null)
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000))
  const [cronInvalid, setCronInvalid] = useState(false)

  // ── Initial load + live config sync via `config:changed` ──
  const loadCfg = useCallback(async () => {
    try {
      const c = await getTransport().call<DreamingConfig>("get_dreaming_config")
      setCfg(c)
    } catch (e) {
      logger.error("settings", "DreamingPanel::load", "Failed to load config", e)
    }
  }, [])

  useEffect(() => {
    Promise.all([
      getTransport().call<DreamingConfig>("get_dreaming_config"),
      getTransport().call<AvailableModel[]>("get_available_models").catch(() => [] as AvailableModel[]),
      getTransport().call<DreamReport | null>("dreaming_last_report").catch(() => null),
      getTransport().call<IdleStatus>("dreaming_idle_status").catch(() => null),
    ])
      .then(([c, models, report, idle]) => {
        setCfg(c)
        setAvailableModels(models)
        setLastReport(report ?? null)
        setIdleStatus(idle ?? null)
        setLoading(false)
      })
      .catch((e: unknown) => {
        logger.error("settings", "DreamingPanel::loadAll", "Initial load failed", e)
        setLoading(false)
      })
  }, [])

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
      void Promise.all([
        getTransport().call<DreamReport | null>("dreaming_last_report").catch(() => null),
        getTransport().call<IdleStatus>("dreaming_idle_status").catch(() => null),
      ]).then(([r, i]) => {
        setLastReport(r ?? null)
        setIdleStatus(i ?? null)
      })
    })
  }, [])

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
    if (saveTimer.current) clearTimeout(saveTimer.current)
    saveTimer.current = setTimeout(async () => {
      setSaving(true)
      try {
        await getTransport().call("save_dreaming_config", { config: next })
        setSaveStatus("saved")
        setTimeout(() => setSaveStatus("idle"), 1500)
      } catch (e) {
        logger.error("settings", "DreamingPanel::save", "Failed to save", e)
        setSaveStatus("failed")
        setTimeout(() => setSaveStatus("idle"), 1500)
        try {
          const fresh = await getTransport().call<DreamingConfig>("get_dreaming_config")
          setCfg(fresh)
        } catch { /* best effort */ }
      } finally {
        setSaving(false)
      }
    }, 500)
  }, [])

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

  if (loading || !cfg) return null

  const masterDisabled = !cfg.enabled

  // Status row idle countdown.
  const idleCountdownSecs =
    cfg.enabled && cfg.idleTrigger.enabled && idleStatus && idleStatus.lastActivityEpochSecs > 0
      ? Math.max(0, idleStatus.lastActivityEpochSecs + idleStatus.idleMinutes * 60 - now)
      : null

  const narrativeModelValue = cfg.narrativeModel ?? NARRATIVE_MODEL_DEFAULT

  return (
    <div className="flex-1 overflow-y-auto p-6 space-y-6">
      {/* ── Status row ── */}
      {cfg.enabled && (
        <div className="rounded-lg border bg-secondary/30 p-3 text-xs space-y-1">
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
          />
        </div>
      </div>

      <div className={masterDisabled ? "pointer-events-none opacity-50" : ""}>
        {/* ── Idle trigger ── */}
        <Section title={t("settings.dreaming.idleTriggerTitle")}>
          <Row
            label={t("settings.dreaming.idleTriggerEnabled")}
            desc={t("settings.dreaming.idleTriggerEnabledDesc")}
            control={
              <Switch
                checked={cfg.idleTrigger.enabled}
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
            <div className="flex gap-2 items-center">
              <ModelSelector
                value={
                  narrativeModelValue === NARRATIVE_MODEL_DEFAULT ? "" : narrativeModelValue
                }
                onChange={(providerId, modelId) =>
                  save({ ...cfg, narrativeModel: `${providerId}:${modelId}` })
                }
                availableModels={availableModels}
                placeholder={t("settings.dreaming.narrativeModelDefault")}
                separator=":"
                className="flex-1"
              />
              {cfg.narrativeModel && (
                <button
                  type="button"
                  className="text-xs text-muted-foreground hover:text-foreground underline"
                  onClick={() => save({ ...cfg, narrativeModel: null })}
                >
                  {t("settings.dreaming.narrativeModelClear")}
                </button>
              )}
            </div>
          </div>
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
        <Input
          type="number"
          min={min}
          max={max}
          step={step}
          value={value}
          onChange={(e) => {
            const raw = Number(e.target.value)
            if (!Number.isFinite(raw)) return
            const rounded = isInteger ? Math.round(raw) : raw
            const lo = Math.max(min, rounded)
            onChange(max != null ? Math.min(max, lo) : lo)
          }}
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
