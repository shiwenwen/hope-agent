import { useCallback, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Loader2, RefreshCw, Sparkles } from "lucide-react"
import { getTransport } from "@/lib/transport"
import { logger } from "@/lib/logger"

interface LearningOverview {
  windowDays: number
  autoCreatedSkills: number
  userCreatedSkills: number
  skillsActivated: number
  skillsPatched: number
  skillsDiscarded: number
  skillsUsed: number
  recallHits: number
  recallSummaryUsed: number
  profileMemories: number
}

interface TimelinePoint {
  ts: number
  kind: string
  skillId?: string
  source?: string
}

interface SkillUsage {
  skillId: string
  usedCount: number
  lastUsedTs?: number
  createdSource?: string
}

interface RecallStats {
  hits: number
  summarized: number
  windowDays: number
}

const WINDOW_OPTIONS = [7, 14, 30, 60, 90]

export default function LearningTab() {
  const { t } = useTranslation()
  const [windowDays, setWindowDays] = useState(30)
  const [loading, setLoading] = useState(false)
  const [overview, setOverview] = useState<LearningOverview | null>(null)
  const [timeline, setTimeline] = useState<TimelinePoint[]>([])
  const [topSkills, setTopSkills] = useState<SkillUsage[]>([])
  const [recall, setRecall] = useState<RecallStats | null>(null)

  const reload = useCallback(async () => {
    setLoading(true)
    try {
      const [ov, tl, ts, rs] = await Promise.all([
        getTransport().call<LearningOverview>("dashboard_learning_overview", {
          windowDays,
        }),
        getTransport().call<TimelinePoint[]>("dashboard_learning_timeline", {
          windowDays,
        }),
        getTransport().call<SkillUsage[]>("dashboard_top_skills", {
          windowDays,
          limit: 10,
        }),
        getTransport().call<RecallStats>("dashboard_recall_stats", {
          windowDays,
        }),
      ])
      setOverview(ov)
      setTimeline(tl ?? [])
      setTopSkills(ts ?? [])
      setRecall(rs)
    } catch (e) {
      logger.error("dashboard", "LearningTab::load", "Failed to load learning data", e)
    } finally {
      setLoading(false)
    }
  }, [windowDays])

  useEffect(() => {
    reload()
  }, [reload])

  const totalRecall = (recall?.hits ?? 0) + (recall?.summarized ?? 0)
  const summaryPct = totalRecall > 0 ? Math.round(((recall?.summarized ?? 0) / totalRecall) * 100) : 0

  return (
    <div className="flex flex-col gap-4 mt-4">
      <div className="flex items-center justify-between">
        <div className="flex flex-col">
          <h3 className="text-sm font-semibold flex items-center gap-2">
            <Sparkles className="h-4 w-4 text-muted-foreground" />
            {t("dashboard.learning.title")}
          </h3>
          <p className="text-xs text-muted-foreground">{t("dashboard.learning.subtitle")}</p>
        </div>
        <div className="flex gap-2 items-center">
          <div className="flex gap-1">
            {WINDOW_OPTIONS.map((d) => (
              <Button
                key={d}
                size="sm"
                variant={windowDays === d ? "secondary" : "ghost"}
                className="text-xs h-7 px-2"
                onClick={() => setWindowDays(d)}
              >
                {t("dashboard.learning.daysN", { n: d })}
              </Button>
            ))}
          </div>
          <Button size="sm" variant="outline" onClick={reload} disabled={loading}>
            {loading ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <RefreshCw className="h-3.5 w-3.5" />
            )}
          </Button>
        </div>
      </div>

      {/* Overview cards */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
        <OverviewCard
          label={t("dashboard.learning.autoSkills")}
          value={overview?.autoCreatedSkills ?? 0}
          hint={t("dashboard.learning.userSkillsHint", {
            n: overview?.userCreatedSkills ?? 0,
          })}
        />
        <OverviewCard
          label={t("dashboard.learning.activated")}
          value={overview?.skillsActivated ?? 0}
          hint={t("dashboard.learning.patchedHint", {
            n: overview?.skillsPatched ?? 0,
          })}
        />
        <OverviewCard
          label={t("dashboard.learning.recallHits")}
          value={overview?.recallHits ?? 0}
          hint={t("dashboard.learning.recallSummaryHint", {
            n: overview?.recallSummaryUsed ?? 0,
          })}
        />
        <OverviewCard
          label={t("dashboard.learning.profileMemories")}
          value={overview?.profileMemories ?? 0}
          hint={t("dashboard.learning.profileHint")}
        />
      </div>

      {/* Timeline */}
      <div className="border border-border/60 rounded-lg p-4">
        <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground mb-3">
          {t("dashboard.learning.timeline")}
        </h4>
        {timeline.length === 0 ? (
          <div className="text-xs text-muted-foreground text-center py-6">
            {t("dashboard.learning.noEvents")}
          </div>
        ) : (
          <div className="space-y-1 max-h-[240px] overflow-y-auto">
            {timeline.slice().reverse().map((p, i) => (
              <div
                key={`${p.ts}-${i}`}
                className="flex items-center gap-2 text-xs py-1 border-b border-border/20 last:border-0"
              >
                <span className="text-muted-foreground tabular-nums w-32 shrink-0">
                  {new Date(p.ts * 1000).toLocaleString()}
                </span>
                <span
                  className={`px-1.5 py-0.5 rounded text-[10px] font-medium shrink-0 ${kindColor(p.kind)}`}
                >
                  {t(`dashboard.learning.kind.${p.kind}`)}
                </span>
                {p.skillId && (
                  <span className="text-foreground font-medium truncate flex-1">
                    {p.skillId}
                  </span>
                )}
                {p.source && (
                  <span className="text-[10px] text-muted-foreground">{p.source}</span>
                )}
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Top skills */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
        <div className="border border-border/60 rounded-lg p-4">
          <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground mb-3">
            {t("dashboard.learning.topSkills")}
          </h4>
          {topSkills.length === 0 ? (
            <div className="text-xs text-muted-foreground text-center py-6">
              {t("dashboard.learning.noSkillUsage")}
            </div>
          ) : (
            <div className="space-y-1.5">
              {topSkills.map((s) => (
                <div
                  key={s.skillId}
                  className="flex items-center gap-2 text-xs py-1 border-b border-border/20 last:border-0"
                >
                  <span className="flex-1 truncate font-medium">{s.skillId}</span>
                  <span className="text-muted-foreground tabular-nums">
                    {s.usedCount}× · {s.lastUsedTs ? new Date(s.lastUsedTs * 1000).toLocaleDateString() : "—"}
                  </span>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Recall effectiveness */}
        <div className="border border-border/60 rounded-lg p-4">
          <h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground mb-3">
            {t("dashboard.learning.recallEffectiveness")}
          </h4>
          {totalRecall === 0 ? (
            <div className="text-xs text-muted-foreground text-center py-6">
              {t("dashboard.learning.noRecall")}
            </div>
          ) : (
            <div className="space-y-2">
              <div className="flex items-center justify-between text-sm">
                <span>{t("dashboard.learning.recallHits")}</span>
                <span className="font-mono">{recall?.hits ?? 0}</span>
              </div>
              <div className="w-full h-2 bg-secondary/40 rounded-full overflow-hidden">
                <div
                  className="h-full bg-emerald-500 transition-all"
                  style={{ width: `${100 - summaryPct}%` }}
                />
              </div>
              <div className="flex items-center justify-between text-sm">
                <span>{t("dashboard.learning.summarized")}</span>
                <span className="font-mono">{recall?.summarized ?? 0}</span>
              </div>
              <div className="w-full h-2 bg-secondary/40 rounded-full overflow-hidden">
                <div
                  className="h-full bg-sky-500 transition-all"
                  style={{ width: `${summaryPct}%` }}
                />
              </div>
              <div className="text-[10px] text-muted-foreground text-right pt-1">
                {t("dashboard.learning.summaryPct", { pct: summaryPct })}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}

function OverviewCard({
  label,
  value,
  hint,
}: {
  label: string
  value: number
  hint?: string
}) {
  return (
    <div className="border border-border/60 rounded-lg p-3 flex flex-col gap-1">
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className="text-2xl font-semibold tabular-nums">{value}</div>
      {hint && <div className="text-[10px] text-muted-foreground">{hint}</div>}
    </div>
  )
}

function kindColor(kind: string): string {
  switch (kind) {
    case "skill_created":
      return "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400"
    case "skill_activated":
      return "bg-sky-500/10 text-sky-600 dark:text-sky-400"
    case "skill_patched":
      return "bg-amber-500/10 text-amber-600 dark:text-amber-400"
    case "skill_discarded":
      return "bg-red-500/10 text-red-600 dark:text-red-400"
    default:
      return "bg-secondary/40 text-muted-foreground"
  }
}
