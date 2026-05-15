import { useState, useEffect, useMemo, useCallback, useRef } from "react"
import { useTranslation } from "react-i18next"
import {
  Sparkles,
  RotateCcw,
  ChevronDown,
  ChevronRight,
  AlertTriangle,
  Check,
  Loader2,
  X,
} from "lucide-react"
import { Switch } from "@/components/ui/switch"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { IconTip } from "@/components/ui/tooltip"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"

// ──────────────────────────────────────────────────────────────────────
// Types — kept locally; the Rust side serializes camelCase via serde.
// ──────────────────────────────────────────────────────────────────────

interface AutoReviewConfig {
  enabled: boolean
  promotion: "draft" | "auto"
  // Gate 1 (trigger)
  cooldownSecs: number
  tokenThreshold: number
  messageThreshold: number
  toolUseThreshold: number
  correctionSignalEnabled: boolean
  requireToolUse: boolean
  // Gate 2
  minMessageCount: number
  discardBlacklistDays: number
  // Gate 3
  topKForDedup: number
  reviewModel?: string | null
  candidateLimit: number
  timeoutSecs: number
  reviewSystemOverride?: string | null
  extraRejectCategories: string[]
  // Gate 4
  minReuseProbability: number
  // Gate 5
  sessionRecapThreshold: number
  minSteps: number
  maxSteps: number
  // Curator
  autoCuratorEnabled: boolean
  autoCuratorIntervalDays: number
  // Retention
  retentionDays: number
}

interface RecentReject {
  ts?: number | null
  skillId?: string | null
  sessionId?: string | null
  rejectReason?: string | null
  rationale?: string | null
  fireReason?: string | null
}

interface ClusterMember {
  skillId: string
  description: string
  similarityToSeed: number
}

interface MergeProposal {
  id: string
  minSimilarity: number
  members: ClusterMember[]
}

interface CuratorReport {
  proposals: MergeProposal[]
  draftsScanned: number
}

interface SkillEvolutionViewProps {
  autoReviewEnabled: boolean
  autoReviewPromotion: boolean
  onSetAutoReviewEnabled: (v: boolean) => void
  onSetAutoReviewPromotion: (v: boolean) => void
}

type SaveStatus = "idle" | "saving" | "saved" | "failed"

export default function SkillEvolutionView({
  autoReviewEnabled,
  autoReviewPromotion,
  onSetAutoReviewEnabled,
  onSetAutoReviewPromotion,
}: SkillEvolutionViewProps) {
  const { t } = useTranslation()
  const [cfg, setCfg] = useState<AutoReviewConfig | null>(null)
  const [rejects, setRejects] = useState<RecentReject[]>([])
  const [curator, setCurator] = useState<CuratorReport | null>(null)
  const [curatorBusy, setCuratorBusy] = useState(false)
  const [merging, setMerging] = useState<Record<string, boolean>>({})
  const [keepSelections, setKeepSelections] = useState<Record<string, string>>({})
  const [openSection, setOpenSection] = useState<Record<string, boolean>>({
    trigger: false,
    quality: false,
    advanced: false,
  })
  const [advancedUnlocked, setAdvancedUnlocked] = useState(false)
  const [saveStatus, setSaveStatus] = useState<Record<string, SaveStatus>>({})
  const saveTimers = useRef<Record<string, ReturnType<typeof setTimeout>>>({})
  const [confirmResetAll, setConfirmResetAll] = useState(false)

  const reload = useCallback(async () => {
    try {
      const [next, recent] = await Promise.all([
        getTransport().call<AutoReviewConfig>("get_skills_auto_review_config"),
        getTransport().call<RecentReject[]>(
          "get_skills_auto_review_recent_rejects",
          { limit: 20 },
        ),
      ])
      setCfg(next)
      setRejects(recent ?? [])
    } catch (e) {
      logger.error(
        "settings",
        "SkillEvolutionView::reload",
        "Failed to load auto-review config",
        e,
      )
    }
  }, [])

  useEffect(() => {
    // Fire initial fetch via microtask so the eslint rule that flags
    // synchronous setState-in-effect doesn't trip on reload()'s internal
    // setCfg/setRejects calls (they all sit behind an await anyway).
    queueMicrotask(() => {
      void reload()
    })
  }, [reload])

  useEffect(() => {
    const timers = saveTimers.current
    return () => {
      Object.values(timers).forEach((id) => clearTimeout(id))
    }
  }, [])

  const flashSave = useCallback((field: string, status: SaveStatus) => {
    setSaveStatus((s) => ({ ...s, [field]: status }))
    if (status === "saved" || status === "failed") {
      if (saveTimers.current[field]) clearTimeout(saveTimers.current[field])
      saveTimers.current[field] = setTimeout(() => {
        setSaveStatus((s) => ({ ...s, [field]: "idle" }))
      }, 2000)
    }
  }, [])

  const patchField = useCallback(
    async (field: keyof AutoReviewConfig, value: unknown) => {
      flashSave(field, "saving")
      try {
        const next = await getTransport().call<AutoReviewConfig>(
          "set_skills_auto_review_config",
          { patch: { [field]: value } },
        )
        setCfg(next)
        flashSave(field, "saved")
      } catch (e) {
        logger.error(
          "settings",
          "SkillEvolutionView::patchField",
          `Failed to patch ${String(field)}`,
          e,
        )
        flashSave(field, "failed")
      }
    },
    [flashSave],
  )

  const resetField = useCallback(
    async (field: keyof AutoReviewConfig & string) => {
      flashSave(field, "saving")
      try {
        // The Rust API takes snake_case field names.
        const snake = field.replace(/[A-Z]/g, (m) => "_" + m.toLowerCase())
        const next = await getTransport().call<AutoReviewConfig>(
          "reset_skills_auto_review_config",
          { fields: [snake] },
        )
        setCfg(next)
        flashSave(field, "saved")
      } catch (e) {
        logger.error(
          "settings",
          "SkillEvolutionView::resetField",
          `Failed to reset ${String(field)}`,
          e,
        )
        flashSave(field, "failed")
      }
    },
    [flashSave],
  )

  const resetAll = useCallback(async () => {
    try {
      const next = await getTransport().call<AutoReviewConfig>(
        "reset_skills_auto_review_config",
        {},
      )
      setCfg(next)
      setConfirmResetAll(false)
      setAdvancedUnlocked(false)
    } catch (e) {
      logger.error(
        "settings",
        "SkillEvolutionView::resetAll",
        "Failed to reset all",
        e,
      )
    }
  }, [])

  const runCurator = useCallback(async () => {
    setCuratorBusy(true)
    try {
      const next = await getTransport().call<CuratorReport>(
        "run_skills_curator_now",
      )
      setCurator(next)
      // seed `keepSelections` with the cluster's first member as default
      const defaults: Record<string, string> = {}
      for (const p of next.proposals) {
        if (p.members.length > 0) defaults[p.id] = p.members[0].skillId
      }
      setKeepSelections((s) => ({ ...defaults, ...s }))
    } catch (e) {
      logger.error(
        "settings",
        "SkillEvolutionView::runCurator",
        "Failed to run curator",
        e,
      )
    } finally {
      setCuratorBusy(false)
    }
  }, [])

  const applyMerge = useCallback(
    async (proposal: MergeProposal) => {
      const keep = keepSelections[proposal.id] ?? proposal.members[0]?.skillId
      if (!keep) return
      setMerging((m) => ({ ...m, [proposal.id]: true }))
      try {
        await getTransport().call("apply_skills_curator_merge", {
          keepId: keep,
          memberIds: proposal.members.map((m) => m.skillId),
        })
        // Drop the applied proposal locally; user can re-run scan to refresh.
        setCurator((c) =>
          c
            ? {
                ...c,
                proposals: c.proposals.filter((p) => p.id !== proposal.id),
              }
            : c,
        )
      } catch (e) {
        logger.error(
          "settings",
          "SkillEvolutionView::applyMerge",
          "Failed to apply merge",
          e,
        )
      } finally {
        setMerging((m) => ({ ...m, [proposal.id]: false }))
      }
    },
    [keepSelections],
  )

  const refreshRejects = useCallback(async () => {
    flashSave("__refresh", "saving")
    try {
      await reload()
      flashSave("__refresh", "saved")
    } catch (e) {
      logger.error(
        "settings",
        "SkillEvolutionView::refreshRejects",
        "Failed to refresh recent rejects",
        e,
      )
      flashSave("__refresh", "failed")
    }
  }, [flashSave, reload])

  const rejectStats = useMemo(() => {
    const counts: Record<string, number> = {}
    for (const r of rejects) {
      const key = r.rejectReason ?? "unknown"
      counts[key] = (counts[key] ?? 0) + 1
    }
    return counts
  }, [rejects])

  if (!cfg) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
        <Loader2 className="h-4 w-4 animate-spin mr-2" />
        {t("settings.skillsEvolution.loading")}
      </div>
    )
  }

  return (
    <div className="flex-1 min-h-0 overflow-y-auto p-6 space-y-5">
      {/* ── Hero: master switch ─────────────────────────────────────── */}
      <div className="overflow-hidden rounded-2xl border border-violet-500/25 bg-gradient-to-br from-violet-500/10 via-fuchsia-500/8 to-pink-500/5 dark:border-violet-400/30 dark:from-violet-500/15 dark:via-fuchsia-500/12 dark:to-pink-500/8">
        <div className="flex items-start gap-4 p-6">
          <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-2xl bg-gradient-to-br from-violet-500 to-fuchsia-500 shadow-lg shadow-violet-500/30">
            <Sparkles className="h-5 w-5 text-white" />
          </div>
          <div className="flex-1 min-w-0">
            <div className="flex flex-wrap items-center gap-2 mb-1.5">
              <h3 className="text-base font-semibold text-foreground">
                {t("settings.skillsEvolutionHero.title")}
              </h3>
              <span
                className={cn(
                  "inline-flex items-center gap-1.5 text-[10px] px-2 py-0.5 rounded-full font-medium",
                  autoReviewEnabled
                    ? "bg-emerald-500/15 text-emerald-700 dark:text-emerald-400"
                    : "bg-muted text-muted-foreground",
                )}
              >
                <span
                  className={cn(
                    "h-1.5 w-1.5 rounded-full",
                    autoReviewEnabled
                      ? "bg-emerald-500 animate-pulse"
                      : "bg-muted-foreground/40",
                  )}
                />
                {autoReviewEnabled
                  ? t("settings.skillsEvolutionHero.statusOn")
                  : t("settings.skillsEvolutionHero.statusOff")}
              </span>
            </div>
            <p className="text-sm leading-relaxed text-muted-foreground">
              {t("settings.skillsEvolutionHero.body")}
            </p>
            <p className="mt-2 text-xs text-muted-foreground/80">
              {t("settings.skillsEvolutionHero.note")}
            </p>
          </div>
          <Switch
            checked={autoReviewEnabled}
            onCheckedChange={onSetAutoReviewEnabled}
            className="mt-1 shrink-0 data-[state=checked]:bg-gradient-to-r data-[state=checked]:from-violet-500 data-[state=checked]:to-fuchsia-500"
          />
        </div>
      </div>

      {/* ── Promotion toggle ─────────────────────────────────────────── */}
      <div
        className={cn(
          "rounded-xl border border-border bg-card/50 p-4 transition-opacity",
          !autoReviewEnabled && "opacity-50",
        )}
      >
        <div className="flex items-start justify-between gap-4">
          <div className="min-w-0">
            <div className="text-sm font-medium text-foreground">
              {t("settings.skillsAutoReview.label")}
            </div>
            <div className="mt-0.5 text-xs text-muted-foreground">
              {t("settings.skillsAutoReview.description")}
            </div>
            <div className="mt-0.5 text-xs text-muted-foreground/70">
              {t("settings.skillsAutoReview.hint")}
            </div>
          </div>
          <Switch
            checked={autoReviewPromotion}
            onCheckedChange={onSetAutoReviewPromotion}
            disabled={!autoReviewEnabled}
            className="shrink-0"
          />
        </div>
      </div>

      {/* ── Recent rejects card ─────────────────────────────────────── */}
      <div
        className={cn(
          "rounded-xl border border-border bg-card/50 p-4",
          !autoReviewEnabled && "opacity-50",
        )}
      >
        <div className="flex items-center justify-between mb-2">
          <div className="text-sm font-medium text-foreground">
            {t("settings.skillsEvolution.recentRejects.title")}
          </div>
          <Button
            variant="ghost"
            size="sm"
            className="h-7 px-2 text-xs"
            onClick={refreshRejects}
            disabled={!autoReviewEnabled || saveStatus.__refresh === "saving"}
          >
            {saveStatus.__refresh === "saving" ? (
              <Loader2 className="h-3 w-3 animate-spin mr-1" />
            ) : (
              <RotateCcw className="h-3 w-3 mr-1" />
            )}
            {t("settings.skillsEvolution.refresh")}
          </Button>
        </div>
        {rejects.length === 0 ? (
          <div className="text-xs text-muted-foreground py-2">
            {t("settings.skillsEvolution.recentRejects.empty")}
          </div>
        ) : (
          <>
            <div className="flex flex-wrap gap-1.5 mb-2">
              {Object.entries(rejectStats).map(([reason, n]) => (
                <span
                  key={reason}
                  className="inline-flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded bg-muted/60 text-muted-foreground"
                >
                  {translateReason(t, reason)}
                  <span className="font-mono text-foreground">{n}</span>
                </span>
              ))}
            </div>
            <div className="space-y-1 max-h-40 overflow-y-auto pr-1">
              {rejects.slice(0, 10).map((r, i) => (
                <div
                  key={i}
                  className="text-xs text-muted-foreground flex items-start gap-2"
                >
                  <span className="font-mono text-foreground/80 shrink-0">
                    {translateReason(t, r.rejectReason ?? "unknown")}
                  </span>
                  {r.rationale && (
                    <span className="truncate">{r.rationale}</span>
                  )}
                </div>
              ))}
            </div>
          </>
        )}
      </div>

      {/* ── Curator (draft consolidation) ───────────────────────────── */}
      <div
        className={cn(
          "rounded-xl border border-border bg-card/50 p-4",
          !autoReviewEnabled && "opacity-50",
        )}
      >
        <div className="flex items-center justify-between mb-2">
          <div className="min-w-0">
            <div className="text-sm font-medium text-foreground">
              {t("settings.skillsEvolution.curator.title")}
            </div>
            <div className="text-[11px] text-muted-foreground">
              {t("settings.skillsEvolution.curator.help")}
            </div>
          </div>
          <Button
            variant="ghost"
            size="sm"
            className="h-7 px-2 text-xs shrink-0"
            onClick={runCurator}
            disabled={!autoReviewEnabled || curatorBusy}
          >
            {curatorBusy ? (
              <Loader2 className="h-3 w-3 animate-spin mr-1" />
            ) : (
              <Sparkles className="h-3 w-3 mr-1" />
            )}
            {t("settings.skillsEvolution.curator.run")}
          </Button>
        </div>
        {curator == null ? (
          <div className="text-xs text-muted-foreground py-2">
            {t("settings.skillsEvolution.curator.idle")}
          </div>
        ) : curator.proposals.length === 0 ? (
          <div className="text-xs text-muted-foreground py-2">
            {t("settings.skillsEvolution.curator.noClusters", {
              count: curator.draftsScanned,
            })}
          </div>
        ) : (
          <div className="space-y-2">
            <div className="text-[11px] text-muted-foreground">
              {t("settings.skillsEvolution.curator.foundN", {
                proposals: curator.proposals.length,
                scanned: curator.draftsScanned,
              })}
            </div>
            {curator.proposals.map((p) => {
              const keep = keepSelections[p.id] ?? p.members[0]?.skillId
              return (
                <div
                  key={p.id}
                  className="rounded-md border border-border/60 bg-background/40 p-3"
                >
                  <div className="text-[11px] text-muted-foreground mb-2">
                    {t("settings.skillsEvolution.curator.clusterMeta", {
                      members: p.members.length,
                      similarity: p.minSimilarity.toFixed(2),
                    })}
                  </div>
                  <div className="space-y-1.5">
                    {p.members.map((m) => (
                      <label
                        key={m.skillId}
                        className={cn(
                          "flex items-start gap-2 p-1.5 rounded-md text-xs cursor-pointer hover:bg-muted/50",
                          keep === m.skillId && "bg-emerald-500/10",
                        )}
                      >
                        <input
                          type="radio"
                          name={`keep-${p.id}`}
                          checked={keep === m.skillId}
                          onChange={() =>
                            setKeepSelections((s) => ({
                              ...s,
                              [p.id]: m.skillId,
                            }))
                          }
                          className="mt-0.5 shrink-0"
                        />
                        <div className="min-w-0 flex-1">
                          <div className="font-mono text-foreground truncate">
                            {m.skillId}
                          </div>
                          <div className="text-muted-foreground truncate">
                            {m.description}
                          </div>
                        </div>
                      </label>
                    ))}
                  </div>
                  <div className="mt-2 flex justify-end gap-2">
                    <Button
                      size="sm"
                      variant="default"
                      className="h-7 px-3 text-xs"
                      onClick={() => applyMerge(p)}
                      disabled={!keep || merging[p.id]}
                    >
                      {merging[p.id] ? (
                        <Loader2 className="h-3 w-3 animate-spin mr-1" />
                      ) : null}
                      {t("settings.skillsEvolution.curator.apply", {
                        n: p.members.length - 1,
                      })}
                    </Button>
                  </div>
                </div>
              )
            })}
          </div>
        )}
      </div>

      {/* ── Trigger section ─────────────────────────────────────────── */}
      <Section
        open={openSection.trigger}
        onToggle={() =>
          setOpenSection((s) => ({ ...s, trigger: !s.trigger }))
        }
        disabled={!autoReviewEnabled}
        title={t("settings.skillsEvolution.sections.trigger.title")}
        subtitle={t("settings.skillsEvolution.sections.trigger.subtitle")}
      >
        <BoolField
          label={t("settings.skillsEvolution.fields.requireToolUse.label")}
          help={t("settings.skillsEvolution.fields.requireToolUse.help")}
          value={cfg.requireToolUse}
          onChange={(v) => void patchField("requireToolUse", v)}
          onReset={() => void resetField("requireToolUse")}
          status={saveStatus.requireToolUse}
        />
        <BoolField
          label={t(
            "settings.skillsEvolution.fields.correctionSignalEnabled.label",
          )}
          help={t(
            "settings.skillsEvolution.fields.correctionSignalEnabled.help",
          )}
          value={cfg.correctionSignalEnabled}
          onChange={(v) => void patchField("correctionSignalEnabled", v)}
          onReset={() => void resetField("correctionSignalEnabled")}
          status={saveStatus.correctionSignalEnabled}
        />
        <NumField
          label={t("settings.skillsEvolution.fields.toolUseThreshold.label")}
          help={t("settings.skillsEvolution.fields.toolUseThreshold.help")}
          value={cfg.toolUseThreshold}
          onChange={(v) => void patchField("toolUseThreshold", v)}
          onReset={() => void resetField("toolUseThreshold")}
          status={saveStatus.toolUseThreshold}
          min={0}
          step={1}
        />
        <NumField
          label={t("settings.skillsEvolution.fields.cooldownSecs.label")}
          help={t("settings.skillsEvolution.fields.cooldownSecs.help")}
          value={cfg.cooldownSecs}
          onChange={(v) => void patchField("cooldownSecs", v)}
          onReset={() => void resetField("cooldownSecs")}
          status={saveStatus.cooldownSecs}
          min={60}
          step={60}
          unit={t("settings.skillsEvolution.units.seconds")}
        />
        <NumField
          label={t("settings.skillsEvolution.fields.tokenThreshold.label")}
          help={t("settings.skillsEvolution.fields.tokenThreshold.help")}
          value={cfg.tokenThreshold}
          onChange={(v) => void patchField("tokenThreshold", v)}
          onReset={() => void resetField("tokenThreshold")}
          status={saveStatus.tokenThreshold}
          min={1000}
          step={1000}
        />
        <NumField
          label={t("settings.skillsEvolution.fields.messageThreshold.label")}
          help={t("settings.skillsEvolution.fields.messageThreshold.help")}
          value={cfg.messageThreshold}
          onChange={(v) => void patchField("messageThreshold", v)}
          onReset={() => void resetField("messageThreshold")}
          status={saveStatus.messageThreshold}
          min={3}
          step={1}
        />
      </Section>

      {/* ── Quality gates section ───────────────────────────────────── */}
      <Section
        open={openSection.quality}
        onToggle={() =>
          setOpenSection((s) => ({ ...s, quality: !s.quality }))
        }
        disabled={!autoReviewEnabled}
        title={t("settings.skillsEvolution.sections.quality.title")}
        subtitle={t("settings.skillsEvolution.sections.quality.subtitle")}
      >
        <NumField
          label={t("settings.skillsEvolution.fields.minReuseProbability.label")}
          help={t("settings.skillsEvolution.fields.minReuseProbability.help")}
          value={cfg.minReuseProbability}
          onChange={(v) => void patchField("minReuseProbability", v)}
          onReset={() => void resetField("minReuseProbability")}
          status={saveStatus.minReuseProbability}
          min={0}
          max={1}
          step={0.05}
          isFloat
        />
        <NumField
          label={t("settings.skillsEvolution.fields.topKForDedup.label")}
          help={t("settings.skillsEvolution.fields.topKForDedup.help")}
          value={cfg.topKForDedup}
          onChange={(v) => void patchField("topKForDedup", v)}
          onReset={() => void resetField("topKForDedup")}
          status={saveStatus.topKForDedup}
          min={1}
          max={20}
          step={1}
        />
        <NumField
          label={t(
            "settings.skillsEvolution.fields.discardBlacklistDays.label",
          )}
          help={t("settings.skillsEvolution.fields.discardBlacklistDays.help")}
          value={cfg.discardBlacklistDays}
          onChange={(v) => void patchField("discardBlacklistDays", v)}
          onReset={() => void resetField("discardBlacklistDays")}
          status={saveStatus.discardBlacklistDays}
          min={0}
          step={1}
          unit={t("settings.skillsEvolution.units.days")}
        />
        <NumField
          label={t("settings.skillsEvolution.fields.minMessageCount.label")}
          help={t("settings.skillsEvolution.fields.minMessageCount.help")}
          value={cfg.minMessageCount}
          onChange={(v) => void patchField("minMessageCount", v)}
          onReset={() => void resetField("minMessageCount")}
          status={saveStatus.minMessageCount}
          min={0}
          step={1}
        />
        <NumField
          label={t(
            "settings.skillsEvolution.fields.sessionRecapThreshold.label",
          )}
          help={t(
            "settings.skillsEvolution.fields.sessionRecapThreshold.help",
          )}
          value={cfg.sessionRecapThreshold}
          onChange={(v) => void patchField("sessionRecapThreshold", v)}
          onReset={() => void resetField("sessionRecapThreshold")}
          status={saveStatus.sessionRecapThreshold}
          min={0}
          step={1}
        />
        <div className="grid grid-cols-2 gap-3">
          <NumField
            label={t("settings.skillsEvolution.fields.minSteps.label")}
            help={t("settings.skillsEvolution.fields.minSteps.help")}
            value={cfg.minSteps}
            onChange={(v) => void patchField("minSteps", v)}
            onReset={() => void resetField("minSteps")}
            status={saveStatus.minSteps}
            min={0}
            step={1}
          />
          <NumField
            label={t("settings.skillsEvolution.fields.maxSteps.label")}
            help={t("settings.skillsEvolution.fields.maxSteps.help")}
            value={cfg.maxSteps}
            onChange={(v) => void patchField("maxSteps", v)}
            onReset={() => void resetField("maxSteps")}
            status={saveStatus.maxSteps}
            min={1}
            step={1}
          />
        </div>
      </Section>

      {/* ── Advanced section (double-gate confirm) ──────────────────── */}
      <Section
        open={openSection.advanced}
        onToggle={() => {
          if (!openSection.advanced && !advancedUnlocked) {
            if (
              window.confirm(
                t("settings.skillsEvolution.sections.advanced.confirm"),
              )
            ) {
              setAdvancedUnlocked(true)
              setOpenSection((s) => ({ ...s, advanced: true }))
            }
            return
          }
          setOpenSection((s) => ({ ...s, advanced: !s.advanced }))
        }}
        disabled={!autoReviewEnabled}
        title={t("settings.skillsEvolution.sections.advanced.title")}
        subtitle={t("settings.skillsEvolution.sections.advanced.subtitle")}
        warning
      >
        <StringField
          label={t("settings.skillsEvolution.fields.reviewModel.label")}
          help={t("settings.skillsEvolution.fields.reviewModel.help")}
          value={cfg.reviewModel ?? ""}
          onChange={(v) => void patchField("reviewModel", v || null)}
          onReset={() => void resetField("reviewModel")}
          status={saveStatus.reviewModel}
          placeholder="anthropic:claude-haiku-4-5"
        />
        <ListField
          label={t(
            "settings.skillsEvolution.fields.extraRejectCategories.label",
          )}
          help={t(
            "settings.skillsEvolution.fields.extraRejectCategories.help",
          )}
          value={cfg.extraRejectCategories ?? []}
          onChange={(v) => void patchField("extraRejectCategories", v)}
          onReset={() => void resetField("extraRejectCategories")}
          status={saveStatus.extraRejectCategories}
        />
        <TextField
          label={t("settings.skillsEvolution.fields.reviewSystemOverride.label")}
          help={t("settings.skillsEvolution.fields.reviewSystemOverride.help")}
          value={cfg.reviewSystemOverride ?? ""}
          onChange={(v) => void patchField("reviewSystemOverride", v || null)}
          onReset={() => void resetField("reviewSystemOverride")}
          status={saveStatus.reviewSystemOverride}
        />
      </Section>

      {/* ── Reset all ────────────────────────────────────────────────── */}
      <div className="flex justify-end pt-2">
        {confirmResetAll ? (
          <div className="flex items-center gap-2">
            <span className="text-xs text-destructive">
              {t("settings.skillsEvolution.resetAll.confirm")}
            </span>
            <Button
              variant="destructive"
              size="sm"
              className="h-7 px-3"
              onClick={resetAll}
            >
              {t("settings.skillsEvolution.resetAll.yes")}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              className="h-7 px-3"
              onClick={() => setConfirmResetAll(false)}
            >
              {t("settings.skillsEvolution.resetAll.no")}
            </Button>
          </div>
        ) : (
          <Button
            variant="ghost"
            size="sm"
            className="h-7 px-3 text-xs text-muted-foreground hover:text-destructive"
            onClick={() => setConfirmResetAll(true)}
          >
            <RotateCcw className="h-3.5 w-3.5 mr-1.5" />
            {t("settings.skillsEvolution.resetAll.label")}
          </Button>
        )}
      </div>
    </div>
  )
}

// ──────────────────────────────────────────────────────────────────────
// Subcomponents
// ──────────────────────────────────────────────────────────────────────

function Section({
  open,
  onToggle,
  title,
  subtitle,
  children,
  disabled,
  warning,
}: {
  open: boolean
  onToggle: () => void
  title: string
  subtitle: string
  children: React.ReactNode
  disabled?: boolean
  warning?: boolean
}) {
  return (
    <div
      className={cn(
        "rounded-xl border border-border bg-card/50 overflow-hidden",
        disabled && "opacity-50 pointer-events-none",
      )}
    >
      <button
        type="button"
        className="w-full flex items-start gap-3 p-4 text-left hover:bg-muted/40"
        onClick={onToggle}
      >
        <div className="mt-0.5 shrink-0 text-muted-foreground">
          {open ? (
            <ChevronDown className="h-4 w-4" />
          ) : (
            <ChevronRight className="h-4 w-4" />
          )}
        </div>
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 text-sm font-medium text-foreground">
            {title}
            {warning && (
              <AlertTriangle className="h-3.5 w-3.5 text-amber-500" />
            )}
          </div>
          <div className="mt-0.5 text-xs text-muted-foreground">{subtitle}</div>
        </div>
      </button>
      {open && <div className="border-t border-border p-4 space-y-3">{children}</div>}
    </div>
  )
}

function FieldRow({
  label,
  help,
  status,
  onReset,
  children,
}: {
  label: string
  help: string
  status?: SaveStatus
  onReset: () => void
  children: React.ReactNode
}) {
  const { t } = useTranslation()
  return (
    <div className="space-y-1">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="text-xs font-medium text-foreground">{label}</div>
          <div className="text-[11px] text-muted-foreground">{help}</div>
        </div>
        <div className="flex items-center gap-1.5 shrink-0">
          <StatusBadge status={status} />
          <IconTip label={t("settings.skillsEvolution.fieldReset")}>
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7 text-muted-foreground/50 hover:text-foreground"
              onClick={onReset}
            >
              <RotateCcw className="h-3.5 w-3.5" />
            </Button>
          </IconTip>
        </div>
      </div>
      {children}
    </div>
  )
}

function StatusBadge({ status }: { status?: SaveStatus }) {
  if (!status || status === "idle") return null
  if (status === "saving") return <Loader2 className="h-3 w-3 animate-spin text-muted-foreground" />
  if (status === "saved")
    return <Check className="h-3 w-3 text-emerald-500" />
  return <X className="h-3 w-3 text-destructive" />
}

function BoolField({
  label,
  help,
  value,
  onChange,
  onReset,
  status,
}: {
  label: string
  help: string
  value: boolean
  onChange: (v: boolean) => void
  onReset: () => void
  status?: SaveStatus
}) {
  return (
    <FieldRow label={label} help={help} status={status} onReset={onReset}>
      <Switch checked={value} onCheckedChange={onChange} />
    </FieldRow>
  )
}

function NumField({
  label,
  help,
  value,
  onChange,
  onReset,
  status,
  min,
  max,
  step,
  unit,
  isFloat,
}: {
  label: string
  help: string
  value: number
  onChange: (v: number) => void
  onReset: () => void
  status?: SaveStatus
  min?: number
  max?: number
  step?: number
  unit?: string
  isFloat?: boolean
}) {
  const [local, setLocal] = useState(String(value))
  useEffect(() => {
    setLocal(String(value))
  }, [value])
  return (
    <FieldRow label={label} help={help} status={status} onReset={onReset}>
      <div className="flex items-center gap-2">
        <Input
          type="number"
          value={local}
          min={min}
          max={max}
          step={step}
          onChange={(e) => setLocal(e.target.value)}
          onBlur={() => {
            const n = isFloat ? parseFloat(local) : parseInt(local, 10)
            if (Number.isFinite(n)) onChange(n)
            else setLocal(String(value))
          }}
          className="h-8 w-32 font-mono"
        />
        {unit && <span className="text-xs text-muted-foreground">{unit}</span>}
      </div>
    </FieldRow>
  )
}

function StringField({
  label,
  help,
  value,
  onChange,
  onReset,
  status,
  placeholder,
}: {
  label: string
  help: string
  value: string
  onChange: (v: string) => void
  onReset: () => void
  status?: SaveStatus
  placeholder?: string
}) {
  const [local, setLocal] = useState(value)
  useEffect(() => {
    setLocal(value)
  }, [value])
  return (
    <FieldRow label={label} help={help} status={status} onReset={onReset}>
      <Input
        value={local}
        placeholder={placeholder}
        onChange={(e) => setLocal(e.target.value)}
        onBlur={() => {
          if (local !== value) onChange(local)
        }}
        className="h-8 font-mono text-xs"
      />
    </FieldRow>
  )
}

function TextField({
  label,
  help,
  value,
  onChange,
  onReset,
  status,
}: {
  label: string
  help: string
  value: string
  onChange: (v: string) => void
  onReset: () => void
  status?: SaveStatus
}) {
  const [local, setLocal] = useState(value)
  useEffect(() => {
    setLocal(value)
  }, [value])
  return (
    <FieldRow label={label} help={help} status={status} onReset={onReset}>
      <Textarea
        value={local}
        rows={8}
        onChange={(e) => setLocal(e.target.value)}
        onBlur={() => {
          if (local !== value) onChange(local)
        }}
        className="font-mono text-xs"
      />
      <div className="text-[10px] text-muted-foreground/70 text-right">
        {local.length}
      </div>
    </FieldRow>
  )
}

function ListField({
  label,
  help,
  value,
  onChange,
  onReset,
  status,
}: {
  label: string
  help: string
  value: string[]
  onChange: (v: string[]) => void
  onReset: () => void
  status?: SaveStatus
}) {
  const [draft, setDraft] = useState("")
  return (
    <FieldRow label={label} help={help} status={status} onReset={onReset}>
      <div className="flex flex-wrap gap-1.5 items-center">
        {value.map((item, i) => (
          <span
            key={i}
            className="inline-flex items-center gap-1 text-xs px-2 py-0.5 rounded-md bg-muted text-foreground"
          >
            {item}
            <button
              type="button"
              className="text-muted-foreground hover:text-destructive"
              onClick={() => onChange(value.filter((_, j) => j !== i))}
            >
              <X className="h-3 w-3" />
            </button>
          </span>
        ))}
        <Input
          value={draft}
          placeholder="add..."
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && draft.trim()) {
              const trimmed = draft.trim()
              if (!value.includes(trimmed)) {
                onChange([...value, trimmed])
              }
              setDraft("")
              e.preventDefault()
            }
          }}
          className="h-7 w-32 text-xs"
        />
      </div>
    </FieldRow>
  )
}

// Localized reason label. Falls back to the stable identifier when no
// translation is registered for the reason — keeps the UI honest about
// what the gate actually emitted.
function translateReason(
  t: (k: string) => string,
  reason: string,
): string {
  const key = `settings.skillsEvolution.rejectReasons.${reason}`
  const translated = t(key)
  return translated === key ? reason : translated
}
