import { useState, useEffect, useMemo } from "react"
import { getTransport } from "@/lib/transport-provider"
import { convertFileSrc } from "@tauri-apps/api/core"
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
import { Switch } from "@/components/ui/switch"
import { X, Bot } from "lucide-react"
import type { CronJob, CronSchedule } from "./CronJobForm.types"

import type { CronFrequency } from "./CronJobForm.types"
import {
  parseCronToVisual,
  buildCronFromVisual,
  toLocalDatetimeString,
} from "./cronHelpers"
import CronExpressionBuilder from "./CronExpressionBuilder"
import type { AgentInfo } from "@/types/chat"

// ── Form Props ────────────────────────────────────────────────────

interface CronJobFormProps {
  job?: CronJob | null
  defaultDate?: Date | null
  onSave: () => void
  onCancel: () => void
}

export default function CronJobForm({ job, defaultDate, onSave, onCancel }: CronJobFormProps) {
  const { t } = useTranslation()
  const isEditing = !!job

  // Form state
  const [name, setName] = useState(job?.name ?? "")
  const [description, setDescription] = useState(job?.description ?? "")
  const [scheduleType, setScheduleType] = useState<"at" | "every" | "cron">(
    job?.schedule.type ?? "cron",
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
    () =>
      parseCronToVisual(
        job?.schedule.type === "cron" ? (job.schedule.expression ?? "") : "0 0 9 * * *",
      ),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  )
  const [cronFreq, setCronFreq] = useState<CronFrequency>(initVisual.freq)
  const [cronHour, setCronHour] = useState(initVisual.hour)
  const [cronMinute, setCronMinute] = useState(initVisual.minute)
  const [cronWeekdays, setCronWeekdays] = useState<boolean[]>(initVisual.weekdays)
  const [cronMonthDay, setCronMonthDay] = useState(initVisual.monthDay)
  const [cronRawExpr, setCronRawExpr] = useState(
    job?.schedule.type === "cron" ? (job.schedule.expression ?? "0 0 9 * * *") : "0 0 9 * * *",
  )

  // Sync visual -> raw expression (for preview and saving)
  const cronExpression = useMemo(
    () =>
      buildCronFromVisual(cronFreq, cronHour, cronMinute, cronWeekdays, cronMonthDay, cronRawExpr),
    [cronFreq, cronHour, cronMinute, cronWeekdays, cronMonthDay, cronRawExpr],
  )

  const [message, setMessage] = useState(job?.payload.prompt ?? "")
  const [agentId, setAgentId] = useState(job?.payload.agentId ?? "default")
  const [maxFailures, setMaxFailures] = useState(String(job?.maxFailures ?? 5))
  const [notifyOnComplete, setNotifyOnComplete] = useState(job?.notifyOnComplete ?? true)
  const [agents, setAgents] = useState<AgentInfo[]>([])
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState("")

  useEffect(() => {
    getTransport().call<AgentInfo[]>("list_agents")
      .then(setAgents)
      .catch(() => {})
  }, [])

  function toggleWeekday(idx: number) {
    setCronWeekdays((prev) => {
      const next = [...prev]
      next[idx] = !next[idx]
      return next
    })
  }

  async function handleSave() {
    if (!name.trim()) {
      setError(t("cron.errorNameRequired"))
      return
    }
    if (!message.trim()) {
      setError(t("cron.errorMessageRequired"))
      return
    }

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
          notifyOnComplete,
        }
        await getTransport().call("cron_update_job", { job: updated })
      } else {
        const schedule = buildSchedule()
        await getTransport().call("cron_create_job", {
          job: {
            name: name.trim(),
            description: description.trim() || null,
            schedule,
            payload: { type: "agentTurn", prompt: message.trim(), agentId: agentId || null },
            maxFailures: parseInt(maxFailures) || 5,
            notifyOnComplete,
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
        const multiplier =
          intervalUnit === "day" ? 86400000 : intervalUnit === "hour" ? 3600000 : 60000
        return { type: "every", intervalMs: Math.max(60000, num * multiplier) }
      }
      case "cron":
        return { type: "cron", expression: cronExpression, timezone: null }
    }
  }

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
            <label className="text-xs font-medium text-muted-foreground mb-1 block">
              {t("cron.name")}
            </label>
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={t("cron.namePlaceholder")}
            />
          </div>

          {/* Description */}
          <div>
            <label className="text-xs font-medium text-muted-foreground mb-1 block">
              {t("cron.description")}
            </label>
            <Input
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder={t("cron.descriptionPlaceholder")}
            />
          </div>

          {/* Schedule Type */}
          <div>
            <label className="text-xs font-medium text-muted-foreground mb-1 block">
              {t("cron.schedule")}
            </label>
            <Select
              value={scheduleType}
              onValueChange={(v) => setScheduleType(v as "at" | "every" | "cron")}
            >
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

          {/* Schedule Config -- One-time */}
          {scheduleType === "at" && (
            <div>
              <label className="text-xs font-medium text-muted-foreground mb-1 block">
                {t("cron.dateTime")}
              </label>
              <Input
                type="datetime-local"
                value={timestamp}
                onChange={(e) => setTimestamp(e.target.value)}
              />
            </div>
          )}

          {/* Schedule Config -- Fixed interval */}
          {scheduleType === "every" && (
            <div className="flex gap-2">
              <div className="flex-1">
                <label className="text-xs font-medium text-muted-foreground mb-1 block">
                  {t("cron.interval")}
                </label>
                <Input
                  type="number"
                  min="1"
                  value={intervalValue}
                  onChange={(e) => setIntervalValue(e.target.value)}
                />
              </div>
              <div className="w-28">
                <label className="text-xs font-medium text-muted-foreground mb-1 block">
                  {t("cron.unit")}
                </label>
                <Select
                  value={intervalUnit}
                  onValueChange={(v) => setIntervalUnit(v as "min" | "hour" | "day")}
                >
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

          {/* Schedule Config -- Cron (visual builder + raw editor) */}
          {scheduleType === "cron" && (
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
          )}

          {/* Message */}
          <div>
            <label className="text-xs font-medium text-muted-foreground mb-1 block">
              {t("cron.message")}
            </label>
            <Textarea
              value={message}
              onChange={(e) => setMessage(e.target.value)}
              placeholder={t("cron.messagePlaceholder")}
              rows={3}
            />
          </div>

          {/* Agent */}
          <div>
            <label className="text-xs font-medium text-muted-foreground mb-1 block">
              {t("cron.agent")}
            </label>
            <Select value={agentId} onValueChange={setAgentId}>
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {agents.map((a) => (
                  <SelectItem key={a.id} value={a.id}>
                    <div className="flex items-center gap-2">
                      <div className="w-5 h-5 rounded-full bg-primary/15 flex items-center justify-center text-primary shrink-0 text-[10px] overflow-hidden">
                        {a.avatar ? (
                          <img
                            src={a.avatar.startsWith("/") ? convertFileSrc(a.avatar) : a.avatar}
                            className="w-full h-full object-cover"
                            alt=""
                          />
                        ) : a.emoji ? (
                          <span>{a.emoji}</span>
                        ) : (
                          <Bot className="h-3 w-3" />
                        )}
                      </div>
                      <span>{a.name}</span>
                    </div>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          {/* Max Failures */}
          <div>
            <label className="text-xs font-medium text-muted-foreground mb-1 block">
              {t("cron.maxFailures")}
            </label>
            <Input
              type="number"
              min="1"
              max="100"
              value={maxFailures}
              onChange={(e) => setMaxFailures(e.target.value)}
            />
          </div>

          {/* Notify on complete */}
          <div className="flex items-center justify-between">
            <div>
              <label className="text-xs font-medium text-muted-foreground block">
                {t("notification.cronNotify")}
              </label>
              <p className="text-xs text-muted-foreground/70 mt-0.5">
                {t("notification.cronNotifyDesc")}
              </p>
            </div>
            <Switch checked={notifyOnComplete} onCheckedChange={setNotifyOnComplete} />
          </div>

          {/* Error */}
          {error && <p className="text-xs text-red-500">{error}</p>}
        </div>

        {/* Footer */}
        <div className="flex justify-end gap-2 px-5 py-4 border-t border-border">
          <Button variant="outline" size="sm" onClick={onCancel}>
            {t("common.cancel")}
          </Button>
          <Button size="sm" onClick={handleSave} disabled={saving}>
            {saving ? t("common.saving") : isEditing ? t("common.save") : t("cron.create")}
          </Button>
        </div>
      </div>
    </div>
  )
}
