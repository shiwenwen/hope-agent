import { useState, useEffect, useMemo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { X, Code2 } from "lucide-react"
import { cn } from "@/lib/utils"

// ── Types ─────────────────────────────────────────────────────────

export interface CronSchedule {
  type: "at" | "every" | "cron"
  timestamp?: string
  intervalMs?: number
  expression?: string
  timezone?: string | null
}

export interface CronPayload {
  type: "agentTurn"
  prompt: string
  agentId?: string | null
}

export interface CronJob {
  id: string
  name: string
  description?: string | null
  schedule: CronSchedule
  payload: CronPayload
  status: "active" | "paused" | "disabled" | "completed" | "missed"
  nextRunAt?: string | null
  lastRunAt?: string | null
  runningAt?: string | null
  consecutiveFailures: number
  maxFailures: number
  createdAt: string
  updatedAt: string
}

export interface CronRunLog {
  id: number
  jobId: string
  sessionId: string
  status: string
  startedAt: string
  finishedAt?: string | null
  durationMs?: number | null
  resultPreview?: string | null
  error?: string | null
}

export interface CalendarEvent {
  jobId: string
  jobName: string
  scheduledAt: string
  status: "active" | "paused" | "disabled" | "completed" | "missed"
  runLog?: CronRunLog | null
}

interface AgentInfo {
  id: string
  name: string
  emoji?: string | null
}

// ── Form Props ────────────────────────────────────────────────────

interface CronJobFormProps {
  job?: CronJob | null
  defaultDate?: Date | null
  onSave: () => void
  onCancel: () => void
}

// ── Visual cron frequency modes ──────────────────────────────────

type CronFrequency = "hourly" | "daily" | "weekly" | "monthly" | "custom"

const WEEKDAY_KEYS = ["weekMon", "weekTue", "weekWed", "weekThu", "weekFri", "weekSat", "weekSun"] as const
const WEEKDAY_CRON = [1, 2, 3, 4, 5, 6, 0] // cron weekday values (Mon=1 .. Sun=0)

/** Parse an existing cron expression into visual-builder state (best effort). */
function parseCronToVisual(expr: string): {
  freq: CronFrequency
  hour: string
  minute: string
  weekdays: boolean[]
  monthDay: string
} {
  const defaults = { freq: "daily" as CronFrequency, hour: "09", minute: "00", weekdays: Array(7).fill(false) as boolean[], monthDay: "1" }
  if (!expr) return defaults

  // cron crate uses 7 fields: sec min hour day month weekday [year]
  const parts = expr.trim().split(/\s+/)
  if (parts.length < 6) return { ...defaults, freq: "custom" }

  const [_sec, min, hour, day, _month, weekday] = parts

  const h = hour === "*" ? "09" : hour.padStart(2, "0")
  const m = min === "*" ? "00" : min.padStart(2, "0")

  // hourly: hour=* min=fixed
  if (hour === "*" && day === "*" && weekday === "*") {
    return { ...defaults, freq: "hourly", hour: h, minute: m }
  }

  // weekly: weekday != *
  if (weekday !== "*" && day === "*") {
    const wds = Array(7).fill(false) as boolean[]
    // Parse weekday field like "1", "1,3,5", "1-5"
    for (const seg of weekday.split(",")) {
      if (seg.includes("-")) {
        const [a, b] = seg.split("-").map(Number)
        for (let v = a; v <= b; v++) {
          const idx = WEEKDAY_CRON.indexOf(v)
          if (idx >= 0) wds[idx] = true
        }
      } else {
        const idx = WEEKDAY_CRON.indexOf(Number(seg))
        if (idx >= 0) wds[idx] = true
      }
    }
    return { freq: "weekly", hour: h, minute: m, weekdays: wds, monthDay: "1" }
  }

  // monthly: day != *
  if (day !== "*" && weekday === "*") {
    return { freq: "monthly", hour: h, minute: m, weekdays: defaults.weekdays, monthDay: day }
  }

  // daily: hour fixed, day=*, weekday=*
  if (hour !== "*" && day === "*" && weekday === "*") {
    return { freq: "daily", hour: h, minute: m, weekdays: defaults.weekdays, monthDay: "1" }
  }

  return { ...defaults, freq: "custom" }
}

/** Build cron expression from visual state. */
function buildCronFromVisual(
  freq: CronFrequency,
  hour: string,
  minute: string,
  weekdays: boolean[],
  monthDay: string,
  rawExpr: string,
): string {
  const h = parseInt(hour) || 0
  const m = parseInt(minute) || 0

  switch (freq) {
    case "hourly":
      return `0 ${m} * * * *`
    case "daily":
      return `0 ${m} ${h} * * *`
    case "weekly": {
      const selected = weekdays
        .map((on, i) => (on ? WEEKDAY_CRON[i] : -1))
        .filter((v) => v >= 0)
      if (selected.length === 0) return `0 ${m} ${h} * * *` // fallback daily
      return `0 ${m} ${h} * * ${selected.join(",")}`
    }
    case "monthly": {
      const d = parseInt(monthDay) || 1
      return `0 ${m} ${h} ${d} * *`
    }
    case "custom":
      return rawExpr
  }
}

export default function CronJobForm({ job, defaultDate, onSave, onCancel }: CronJobFormProps) {
  const { t } = useTranslation()
  const isEditing = !!job

  // Form state
  const [name, setName] = useState(job?.name ?? "")
  const [description, setDescription] = useState(job?.description ?? "")
  const [scheduleType, setScheduleType] = useState<"at" | "every" | "cron">(
    job?.schedule.type ?? "cron"
  )
  const [timestamp, setTimestamp] = useState(() => {
    if (job?.schedule.type === "at" && job.schedule.timestamp) {
      return toLocalDatetimeString(job.schedule.timestamp)
    }
    if (defaultDate) {
      return toLocalDatetimeString(defaultDate.toISOString())
    }
    return ""
  })
  const [intervalValue, setIntervalValue] = useState(() => {
    if (job?.schedule.type === "every" && job.schedule.intervalMs) {
      return String(job.schedule.intervalMs / 60000)
    }
    return "60"
  })
  const [intervalUnit, setIntervalUnit] = useState<"min" | "hour" | "day">("min")

  // Visual cron builder state
  const initVisual = useMemo(
    () => parseCronToVisual(job?.schedule.type === "cron" ? job.schedule.expression ?? "" : "0 0 9 * * *"),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  )
  const [cronFreq, setCronFreq] = useState<CronFrequency>(initVisual.freq)
  const [cronHour, setCronHour] = useState(initVisual.hour)
  const [cronMinute, setCronMinute] = useState(initVisual.minute)
  const [cronWeekdays, setCronWeekdays] = useState<boolean[]>(initVisual.weekdays)
  const [cronMonthDay, setCronMonthDay] = useState(initVisual.monthDay)
  const [cronRawExpr, setCronRawExpr] = useState(
    job?.schedule.type === "cron" ? job.schedule.expression ?? "0 0 9 * * *" : "0 0 9 * * *",
  )

  // Sync visual → raw expression (for preview and saving)
  const cronExpression = useMemo(
    () => buildCronFromVisual(cronFreq, cronHour, cronMinute, cronWeekdays, cronMonthDay, cronRawExpr),
    [cronFreq, cronHour, cronMinute, cronWeekdays, cronMonthDay, cronRawExpr],
  )

  const [message, setMessage] = useState(job?.payload.prompt ?? "")
  const [agentId, setAgentId] = useState(job?.payload.agentId ?? "default")
  const [maxFailures, setMaxFailures] = useState(String(job?.maxFailures ?? 5))
  const [agents, setAgents] = useState<AgentInfo[]>([])
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState("")

  useEffect(() => {
    invoke<AgentInfo[]>("list_agents").then(setAgents).catch(() => {})
  }, [])

  function toggleWeekday(idx: number) {
    setCronWeekdays((prev) => {
      const next = [...prev]
      next[idx] = !next[idx]
      return next
    })
  }

  async function handleSave() {
    if (!name.trim()) { setError(t("cron.errorNameRequired")); return }
    if (!message.trim()) { setError(t("cron.errorMessageRequired")); return }

    setSaving(true)
    setError("")

    try {
      if (isEditing && job) {
        const schedule = buildSchedule()
        const updated: CronJob = {
          ...job,
          name: name.trim(),
          description: description.trim() || null,
          schedule,
          payload: { type: "agentTurn", prompt: message.trim(), agentId: agentId || null },
          maxFailures: parseInt(maxFailures) || 5,
        }
        await invoke("cron_update_job", { job: updated })
      } else {
        const schedule = buildSchedule()
        await invoke("cron_create_job", {
          job: {
            name: name.trim(),
            description: description.trim() || null,
            schedule,
            payload: { type: "agentTurn", prompt: message.trim(), agentId: agentId || null },
            maxFailures: parseInt(maxFailures) || 5,
          },
        })
      }
      onSave()
    } catch (e: unknown) {
      setError(String(e))
    } finally {
      setSaving(false)
    }
  }

  function buildSchedule(): CronSchedule {
    switch (scheduleType) {
      case "at":
        return { type: "at", timestamp: new Date(timestamp).toISOString() }
      case "every": {
        const num = parseFloat(intervalValue) || 60
        const multiplier = intervalUnit === "day" ? 86400000 : intervalUnit === "hour" ? 3600000 : 60000
        return { type: "every", intervalMs: Math.max(60000, num * multiplier) }
      }
      case "cron":
        return { type: "cron", expression: cronExpression, timezone: null }
    }
  }

  // ── Hour / Minute options ──────────────────────────────────────
  const hourOptions = Array.from({ length: 24 }, (_, i) => String(i).padStart(2, "0"))
  const minuteOptions = Array.from({ length: 12 }, (_, i) => String(i * 5).padStart(2, "0"))

  return (
    <div className="fixed inset-0 z-50 bg-black/50 flex items-center justify-center p-4">
      <div className="bg-card border border-border rounded-xl shadow-xl w-full max-w-lg max-h-[90vh] overflow-y-auto">
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-border">
          <h3 className="text-base font-medium">
            {isEditing ? t("cron.editJob") : t("cron.newJob")}
          </h3>
          <Button variant="ghost" size="icon" className="h-7 w-7" onClick={onCancel}>
            <X className="h-4 w-4" />
          </Button>
        </div>

        {/* Form */}
        <div className="p-5 space-y-4">
          {/* Name */}
          <div>
            <label className="text-xs font-medium text-muted-foreground mb-1 block">{t("cron.name")}</label>
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={t("cron.namePlaceholder")}
            />
          </div>

          {/* Description */}
          <div>
            <label className="text-xs font-medium text-muted-foreground mb-1 block">{t("cron.description")}</label>
            <Input
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder={t("cron.descriptionPlaceholder")}
            />
          </div>

          {/* Schedule Type */}
          <div>
            <label className="text-xs font-medium text-muted-foreground mb-1 block">{t("cron.schedule")}</label>
            <Select value={scheduleType} onValueChange={(v) => setScheduleType(v as "at" | "every" | "cron")}>
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="at">{t("cron.scheduleAt")}</SelectItem>
                <SelectItem value="every">{t("cron.scheduleEvery")}</SelectItem>
                <SelectItem value="cron">{t("cron.scheduleCron")}</SelectItem>
              </SelectContent>
            </Select>
          </div>

          {/* Schedule Config — One-time */}
          {scheduleType === "at" && (
            <div>
              <label className="text-xs font-medium text-muted-foreground mb-1 block">{t("cron.dateTime")}</label>
              <Input
                type="datetime-local"
                value={timestamp}
                onChange={(e) => setTimestamp(e.target.value)}
              />
            </div>
          )}

          {/* Schedule Config — Fixed interval */}
          {scheduleType === "every" && (
            <div className="flex gap-2">
              <div className="flex-1">
                <label className="text-xs font-medium text-muted-foreground mb-1 block">{t("cron.interval")}</label>
                <Input
                  type="number"
                  min="1"
                  value={intervalValue}
                  onChange={(e) => setIntervalValue(e.target.value)}
                />
              </div>
              <div className="w-28">
                <label className="text-xs font-medium text-muted-foreground mb-1 block">{t("cron.unit")}</label>
                <Select value={intervalUnit} onValueChange={(v) => setIntervalUnit(v as "min" | "hour" | "day")}>
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="min">{t("cron.unitMinutes")}</SelectItem>
                    <SelectItem value="hour">{t("cron.unitHours")}</SelectItem>
                    <SelectItem value="day">{t("cron.unitDays")}</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </div>
          )}

          {/* Schedule Config — Cron (visual builder + raw editor) */}
          {scheduleType === "cron" && (
            <div className="space-y-3">
              {/* Frequency pills */}
              <div>
                <label className="text-xs font-medium text-muted-foreground mb-1.5 block">{t("cron.frequency")}</label>
                <div className="flex flex-wrap gap-1.5">
                  {(["hourly", "daily", "weekly", "monthly", "custom"] as CronFrequency[]).map((f) => (
                    <button
                      key={f}
                      type="button"
                      className={cn(
                        "px-3 py-1 rounded-full text-xs font-medium transition-colors",
                        cronFreq === f
                          ? "bg-primary text-primary-foreground"
                          : "bg-secondary text-secondary-foreground hover:bg-secondary/80",
                      )}
                      onClick={() => setCronFreq(f)}
                    >
                      {t(`cron.freq_${f}`)}
                    </button>
                  ))}
                </div>
              </div>

              {/* Hourly: at minute */}
              {cronFreq === "hourly" && (
                <div className="flex items-center gap-2 text-xs">
                  <span className="text-muted-foreground">{t("cron.atMinute")}</span>
                  <Select value={cronMinute} onValueChange={setCronMinute}>
                    <SelectTrigger className="w-20 h-8 text-xs">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {minuteOptions.map((m) => (
                        <SelectItem key={m} value={m}>{m}</SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <span className="text-muted-foreground">{t("cron.minuteOfHour")}</span>
                </div>
              )}

              {/* Daily: time picker */}
              {cronFreq === "daily" && (
                <div className="flex items-center gap-2 text-xs">
                  <span className="text-muted-foreground">{t("cron.everyDayAt")}</span>
                  <Select value={cronHour} onValueChange={setCronHour}>
                    <SelectTrigger className="w-20 h-8 text-xs">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {hourOptions.map((h) => (
                        <SelectItem key={h} value={h}>{h}</SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <span>:</span>
                  <Select value={cronMinute} onValueChange={setCronMinute}>
                    <SelectTrigger className="w-20 h-8 text-xs">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {minuteOptions.map((m) => (
                        <SelectItem key={m} value={m}>{m}</SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
              )}

              {/* Weekly: weekday toggles + time */}
              {cronFreq === "weekly" && (
                <div className="space-y-2">
                  <div className="flex gap-1">
                    {WEEKDAY_KEYS.map((key, i) => (
                      <button
                        key={key}
                        type="button"
                        className={cn(
                          "flex-1 py-1.5 rounded-md text-xs font-medium transition-colors",
                          cronWeekdays[i]
                            ? "bg-primary text-primary-foreground"
                            : "bg-secondary text-secondary-foreground hover:bg-secondary/80",
                        )}
                        onClick={() => toggleWeekday(i)}
                      >
                        {t(`cron.${key}`)}
                      </button>
                    ))}
                  </div>
                  <div className="flex items-center gap-2 text-xs">
                    <span className="text-muted-foreground">{t("cron.atTime")}</span>
                    <Select value={cronHour} onValueChange={setCronHour}>
                      <SelectTrigger className="w-20 h-8 text-xs">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {hourOptions.map((h) => (
                          <SelectItem key={h} value={h}>{h}</SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <span>:</span>
                    <Select value={cronMinute} onValueChange={setCronMinute}>
                      <SelectTrigger className="w-20 h-8 text-xs">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {minuteOptions.map((m) => (
                          <SelectItem key={m} value={m}>{m}</SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                </div>
              )}

              {/* Monthly: day of month + time */}
              {cronFreq === "monthly" && (
                <div className="space-y-2">
                  <div className="flex items-center gap-2 text-xs">
                    <span className="text-muted-foreground">{t("cron.everyMonthOn")}</span>
                    <Select value={cronMonthDay} onValueChange={setCronMonthDay}>
                      <SelectTrigger className="w-20 h-8 text-xs">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {Array.from({ length: 31 }, (_, i) => String(i + 1)).map((d) => (
                          <SelectItem key={d} value={d}>{d}{t("cron.daySuffix")}</SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                  <div className="flex items-center gap-2 text-xs">
                    <span className="text-muted-foreground">{t("cron.atTime")}</span>
                    <Select value={cronHour} onValueChange={setCronHour}>
                      <SelectTrigger className="w-20 h-8 text-xs">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {hourOptions.map((h) => (
                          <SelectItem key={h} value={h}>{h}</SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <span>:</span>
                    <Select value={cronMinute} onValueChange={setCronMinute}>
                      <SelectTrigger className="w-20 h-8 text-xs">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {minuteOptions.map((m) => (
                          <SelectItem key={m} value={m}>{m}</SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                </div>
              )}

              {/* Custom: raw cron expression */}
              {cronFreq === "custom" && (
                <div>
                  <Input
                    value={cronRawExpr}
                    onChange={(e) => setCronRawExpr(e.target.value)}
                    placeholder="0 0 9 * * *"
                    className="font-mono text-sm"
                  />
                  <p className="text-[10px] text-muted-foreground mt-1">
                    {t("cron.cronHelp")}
                  </p>
                </div>
              )}

              {/* Generated expression preview (non-custom modes) */}
              {cronFreq !== "custom" && (
                <div className="flex items-center gap-2 text-[10px] text-muted-foreground bg-secondary/40 rounded-md px-2.5 py-1.5">
                  <Code2 className="h-3 w-3 shrink-0" />
                  <span className="font-mono">{cronExpression}</span>
                  <button
                    type="button"
                    className="ml-auto text-primary hover:underline shrink-0"
                    onClick={() => {
                      setCronRawExpr(cronExpression)
                      setCronFreq("custom")
                    }}
                  >
                    {t("cron.editExpression")}
                  </button>
                </div>
              )}
            </div>
          )}

          {/* Message */}
          <div>
            <label className="text-xs font-medium text-muted-foreground mb-1 block">{t("cron.message")}</label>
            <Textarea
              value={message}
              onChange={(e) => setMessage(e.target.value)}
              placeholder={t("cron.messagePlaceholder")}
              rows={3}
            />
          </div>

          {/* Agent */}
          <div>
            <label className="text-xs font-medium text-muted-foreground mb-1 block">{t("cron.agent")}</label>
            <Select value={agentId} onValueChange={setAgentId}>
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {agents.map((a) => (
                  <SelectItem key={a.id} value={a.id}>
                    {a.emoji ? `${a.emoji} ` : ""}{a.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          {/* Max Failures */}
          <div>
            <label className="text-xs font-medium text-muted-foreground mb-1 block">{t("cron.maxFailures")}</label>
            <Input
              type="number"
              min="1"
              max="100"
              value={maxFailures}
              onChange={(e) => setMaxFailures(e.target.value)}
            />
          </div>

          {/* Error */}
          {error && (
            <p className="text-xs text-red-500">{error}</p>
          )}
        </div>

        {/* Footer */}
        <div className="flex justify-end gap-2 px-5 py-4 border-t border-border">
          <Button variant="outline" size="sm" onClick={onCancel}>{t("common.cancel")}</Button>
          <Button size="sm" onClick={handleSave} disabled={saving}>
            {saving ? t("common.saving") : isEditing ? t("common.save") : t("cron.create")}
          </Button>
        </div>
      </div>
    </div>
  )
}

// ── Helpers ───────────────────────────────────────────────────────

function toLocalDatetimeString(isoString: string): string {
  try {
    const d = new Date(isoString)
    const pad = (n: number) => String(n).padStart(2, "0")
    return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`
  } catch {
    return ""
  }
}

// ── Shared Status Helpers ─────────────────────────────────────────

export function statusColor(status: string): string {
  switch (status) {
    case "active": return "bg-emerald-500"
    case "paused": return "bg-amber-500"
    case "disabled": return "bg-red-500"
    case "completed": return "bg-gray-400"
    case "missed": return "bg-orange-500"
    default: return "bg-gray-400"
  }
}

export function formatSchedule(schedule: CronSchedule, t: (key: string) => string): string {
  switch (schedule.type) {
    case "at":
      return `${t("cron.scheduleAt")}: ${schedule.timestamp ? new Date(schedule.timestamp).toLocaleString() : ""}`
    case "every": {
      const ms = schedule.intervalMs ?? 0
      const secs = ms / 1000
      if (secs < 3600) return `${t("cron.scheduleEvery")} ${Math.round(secs / 60)} ${t("cron.unitMinutes")}`
      if (secs < 86400) return `${t("cron.scheduleEvery")} ${Math.round(secs / 3600)} ${t("cron.unitHours")}`
      return `${t("cron.scheduleEvery")} ${Math.round(secs / 86400)} ${t("cron.unitDays")}`
    }
    case "cron":
      return `Cron: ${schedule.expression}`
  }
}
