import { useState, useEffect, useMemo } from "react"
import { getTransport } from "@/lib/transport-provider"
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
import { X, Plus, Send, FolderOpen, AlertTriangle } from "lucide-react"
import { AgentSelectDisplay } from "@/components/common/AgentSelectDisplay"
import type { CronDeliveryTarget, CronJob, CronSchedule } from "./CronJobForm.types"

import type { CronFrequency } from "./CronJobForm.types"
import {
  parseCronToVisual,
  buildCronFromVisual,
  toLocalDatetimeString,
} from "./cronHelpers"
import CronExpressionBuilder from "./CronExpressionBuilder"
import type { AgentInfo } from "@/types/chat"
import type { ChannelAccountConfig } from "@/components/settings/channel-panel/types"
import type { ProjectMeta } from "@/types/project"

// Matches the shape returned by `channel_list_sessions` (see
// `src-tauri/src/commands/channel.rs::channel_list_sessions`).
interface ChannelConversationDto {
  id: number
  channelId: string
  accountId: string
  chatId: string
  threadId?: string | null
  sessionId: string
  senderId?: string | null
  senderName?: string | null
  chatType: string
  createdAt: string
  updatedAt: string
}

// ── Form Props ────────────────────────────────────────────────────

interface CronJobFormProps {
  job?: CronJob | null
  defaultDate?: Date | null
  defaultProjectId?: string | null
  onSave: () => void
  onCancel: () => void
}

const AUTO_AGENT_VALUE = "__auto__"
const NO_PROJECT_VALUE = "__none__"

export default function CronJobForm({
  job,
  defaultDate,
  defaultProjectId,
  onSave,
  onCancel,
}: CronJobFormProps) {
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
    const intervalMs =
      job?.schedule.type === "every" ? (job.schedule.intervalMs ?? job.schedule.interval_ms) : null
    if (intervalMs) {
      return String(intervalMs / 60000)
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

  // Timezone for cron schedules: the wall-clock hour/minute fields are
  // interpreted in this IANA zone (DST-aware). Defaults to the browser's
  // detected zone for new jobs so "9am" means the user's 9am, not UTC.
  const [timezone, setTimezone] = useState<string>(() => {
    // Editing an existing cron job: preserve its stored zone EXACTLY. A null/empty
    // stored zone is a deliberate UTC job (the "Omit for UTC" contract) and must
    // NOT fall through to the browser zone — otherwise an unrelated edit (rename,
    // delivery target) would rewrite the zone to the browser's on save and shift
    // every fire's wall-clock by the local UTC offset. null normalizes to "UTC"
    // (semantically identical for the backend; just an explicit, visible value).
    if (job?.schedule.type === "cron") {
      return job.schedule.timezone || "UTC"
    }
    // New job (or converting a non-cron schedule): default to the browser's zone
    // so "9am" means the user's 9am, not UTC.
    try {
      return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC"
    } catch {
      return "UTC"
    }
  })

  // Full IANA zone list for the picker, computed once; falls back gracefully on
  // engines without `Intl.supportedValuesOf`. Always includes UTC.
  const baseTimezones = useMemo<string[]>(() => {
    let zones: string[] = []
    try {
      const supported = (Intl as { supportedValuesOf?: (key: string) => string[] })
        .supportedValuesOf
      if (supported) zones = supported("timeZone")
    } catch {
      zones = []
    }
    const set = new Set(zones)
    set.add("UTC")
    return Array.from(set).sort()
  }, [])

  // Ensure the current selection renders even when it isn't in the standard list
  // (e.g. a backfilled host zone the browser doesn't enumerate) — no full re-sort
  // on every change, just a membership check.
  const timezoneOptions = useMemo<string[]>(
    () => (timezone && !baseTimezones.includes(timezone) ? [timezone, ...baseTimezones] : baseTimezones),
    [baseTimezones, timezone],
  )

  const [message, setMessage] = useState(job?.payload.prompt ?? "")
  const [agentId, setAgentId] = useState(job?.payload.agentId ?? AUTO_AGENT_VALUE)
  const [projectId, setProjectId] = useState(
    job ? (job.projectId ?? NO_PROJECT_VALUE) : (defaultProjectId ?? NO_PROJECT_VALUE),
  )
  const [maxFailures, setMaxFailures] = useState(String(job?.maxFailures ?? 5))
  const [notifyOnComplete, setNotifyOnComplete] = useState(job?.notifyOnComplete ?? true)
  const [prefixDeliveryWithName, setPrefixDeliveryWithName] = useState(
    job?.prefixDeliveryWithName ?? false,
  )
  // C19: per-job timeout override; blank string = use the global default.
  const [jobTimeoutSecs, setJobTimeoutSecs] = useState(
    job?.jobTimeoutSecs ? String(job.jobTimeoutSecs) : "",
  )
  const [deliveryTargets, setDeliveryTargets] = useState<CronDeliveryTarget[]>(
    () => job?.deliveryTargets?.map((t) => ({ ...t })) ?? [],
  )
  const [accounts, setAccounts] = useState<ChannelAccountConfig[]>([])
  const [projects, setProjects] = useState<ProjectMeta[]>([])
  const [conversationsByAccount, setConversationsByAccount] = useState<
    Record<string, ChannelConversationDto[]>
  >({})
  const [agents, setAgents] = useState<AgentInfo[]>([])
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState("")
  const selectedAgent = agentId === AUTO_AGENT_VALUE ? null : agents.find((a) => a.id === agentId)
  const selectedProject =
    projectId === NO_PROJECT_VALUE ? null : projects.find((p) => p.id === projectId)
  const isMissingProject = projectId !== NO_PROJECT_VALUE && !selectedProject

  useEffect(() => {
    getTransport().call<AgentInfo[]>("list_agents")
      .then(setAgents)
      .catch(() => {})

    getTransport().call<ChannelAccountConfig[]>("channel_list_accounts")
      .then((list) => setAccounts(list.filter((a) => a.enabled)))
      .catch(() => {})

    getTransport().call<ProjectMeta[]>("list_projects_cmd", { includeArchived: true })
      .then((list) => setProjects(Array.isArray(list) ? list : []))
      .catch(() => {})
  }, [])

  // Prefetch conversations for accounts already used in existing targets.
  useEffect(() => {
    const needed = new Set(deliveryTargets.map((t) => t.accountId).filter(Boolean))
    needed.forEach((accountId) => {
      if (conversationsByAccount[accountId]) return
      const target = deliveryTargets.find((t) => t.accountId === accountId)
      if (!target) return
      void loadConversationsFor(target.channelId, accountId)
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [deliveryTargets])

  async function loadConversationsFor(channelId: string, accountId: string) {
    if (!channelId || !accountId) return
    try {
      const list = await getTransport().call<ChannelConversationDto[]>(
        "channel_list_sessions",
        { channelId, accountId },
      )
      setConversationsByAccount((prev) => ({ ...prev, [accountId]: list }))
    } catch {
      setConversationsByAccount((prev) => ({ ...prev, [accountId]: [] }))
    }
  }

  function addDeliveryTarget() {
    setDeliveryTargets((prev) => [
      ...prev,
      { channelId: "", accountId: "", chatId: "", threadId: null, label: null },
    ])
  }

  function removeDeliveryTarget(idx: number) {
    setDeliveryTargets((prev) => prev.filter((_, i) => i !== idx))
  }

  function handlePickAccount(idx: number, accountId: string) {
    const account = accounts.find((a) => a.id === accountId)
    if (!account) return
    setDeliveryTargets((prev) =>
      prev.map((t, i) =>
        i === idx
          ? {
              ...t,
              channelId: account.channelId,
              accountId: account.id,
              chatId: "",
              threadId: null,
              label: null,
            }
          : t,
      ),
    )
    void loadConversationsFor(account.channelId, account.id)
  }

  function handlePickConversation(idx: number, conversationId: string) {
    const target = deliveryTargets[idx]
    if (!target) return
    const list = conversationsByAccount[target.accountId] ?? []
    const conv = list.find((c) => String(c.id) === conversationId)
    if (!conv) return
    const displayName =
      conv.senderName && conv.senderName.length > 0 ? conv.senderName : conv.chatId
    setDeliveryTargets((prev) =>
      prev.map((t, i) =>
        i === idx
          ? {
              ...t,
              chatId: conv.chatId,
              threadId: conv.threadId ?? null,
              label: `${conv.channelId} / ${displayName}`,
            }
          : t,
      ),
    )
  }

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
    if (scheduleType === "at" && !timestamp.trim()) {
      // Guard before buildSchedule(): `new Date("").toISOString()` throws a
      // RangeError that would otherwise surface as a raw, unlocalized string.
      setError(t("cron.errorDateRequired"))
      return
    }

    setSaving(true)
    setError("")

    // Only persist fully-configured targets (skip rows still awaiting a chat pick).
    const validTargets = deliveryTargets.filter(
      (t) => t.channelId && t.accountId && t.chatId,
    )

    try {
      if (isEditing && job) {
        const schedule = buildSchedule()
        const updated: CronJob = {
          ...job,
          name: name.trim(),
          description: description.trim() || null,
          projectId: projectId === NO_PROJECT_VALUE ? null : projectId,
          schedule,
          payload: {
            type: "agentTurn",
            prompt: message.trim(),
            agentId: agentId === AUTO_AGENT_VALUE ? null : agentId,
          },
          maxFailures: parseInt(maxFailures) || 5,
          notifyOnComplete,
          deliveryTargets: validTargets,
          prefixDeliveryWithName,
          jobTimeoutSecs: jobTimeoutSecs.trim() ? parseInt(jobTimeoutSecs) || null : null,
        }
        await getTransport().call("cron_update_job", { job: updated })
      } else {
        const schedule = buildSchedule()
        await getTransport().call("cron_create_job", {
          job: {
            name: name.trim(),
            description: description.trim() || null,
            projectId: projectId === NO_PROJECT_VALUE ? null : projectId,
            schedule,
            payload: {
              type: "agentTurn",
              prompt: message.trim(),
              agentId: agentId === AUTO_AGENT_VALUE ? null : agentId,
            },
            maxFailures: parseInt(maxFailures) || 5,
            notifyOnComplete,
            deliveryTargets: validTargets,
            prefixDeliveryWithName,
            jobTimeoutSecs: jobTimeoutSecs.trim() ? parseInt(jobTimeoutSecs) || null : null,
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
        // Math.round: a fractional value (e.g. 1.1 hours) yields a float-precision
        // artifact (1.1 * 3600000 = 3960000.0000000005) that the backend's u64
        // `interval_ms` rejects at deserialization, failing the whole create/update.
        const intervalMs = Math.max(60000, Math.round(num * multiplier))
        const preserveStartAt =
          job?.schedule.type === "every" &&
          (job.schedule.intervalMs ?? job.schedule.interval_ms) === intervalMs
        return {
          type: "every",
          intervalMs,
          startAt: preserveStartAt
            ? ((job.schedule.startAt ?? job.schedule.start_at) ?? null)
            : undefined,
        }
      }
      case "cron":
        return { type: "cron", expression: cronExpression, timezone: timezone || null }
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
            <>
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
              <div>
                <label className="text-xs font-medium text-muted-foreground mb-1 block">
                  {t("cron.timezone")}
                </label>
                <Select value={timezone} onValueChange={setTimezone}>
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {timezoneOptions.map((tz) => (
                      <SelectItem key={tz} value={tz}>
                        {tz}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <p className="text-[11px] text-muted-foreground mt-1">
                  {t("cron.timezoneHint")}
                </p>
              </div>
            </>
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

          {/* Project */}
          <div>
            <label className="text-xs font-medium text-muted-foreground mb-1 flex items-center gap-1.5">
              <FolderOpen className="h-3 w-3" />
              {t("cron.project")}
            </label>
            <Select value={projectId} onValueChange={setProjectId}>
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={NO_PROJECT_VALUE}>{t("cron.noProject")}</SelectItem>
                {isMissingProject && (
                  <SelectItem value={projectId}>{t("cron.missingProject")}</SelectItem>
                )}
                {projects.map((p) => (
                  <SelectItem key={p.id} value={p.id}>
                    <span className="text-xs">
                      {p.emoji ? `${p.emoji} ` : ""}
                      {p.name}
                      {p.archived ? (
                        <span className="text-muted-foreground ml-1">
                          {t("cron.archivedProject")}
                        </span>
                      ) : null}
                    </span>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          {/* Agent */}
          <div>
            <label className="text-xs font-medium text-muted-foreground mb-1 block">
              {t("cron.agent")}
            </label>
            <Select value={agentId} onValueChange={setAgentId}>
              <SelectTrigger>
                {selectedAgent ? <AgentSelectDisplay agent={selectedAgent} /> : <SelectValue />}
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={AUTO_AGENT_VALUE}>{t("cron.autoAgent")}</SelectItem>
                {agents.map((a) => (
                  <SelectItem key={a.id} value={a.id} textValue={a.name}>
                    <AgentSelectDisplay agent={a} />
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

          {/* Per-job timeout override (C19): blank = use the global default */}
          <div>
            <label className="text-xs font-medium text-muted-foreground mb-1 block">
              {t("cron.jobTimeoutOverride")}
            </label>
            <Input
              type="number"
              min="30"
              max="7200"
              placeholder={t("cron.jobTimeoutOverrideHint")}
              value={jobTimeoutSecs}
              onChange={(e) => setJobTimeoutSecs(e.target.value)}
            />
            <p className="text-[10px] text-muted-foreground mt-1">
              {t("cron.jobTimeoutOverrideHint")}
            </p>
          </div>

          {/* Delivery targets — fan-out job result to IM channel conversations */}
          <div className="space-y-2">
            <div className="flex items-center justify-between">
              <div>
                <label className="text-xs font-medium text-muted-foreground block flex items-center gap-1.5">
                  <Send className="h-3 w-3" />
                  {t("cron.deliveryTargets")}
                </label>
                <p className="text-xs text-muted-foreground/70 mt-0.5">
                  {t("cron.deliveryTargetsDesc")}
                </p>
              </div>
              <Button
                variant="outline"
                size="sm"
                type="button"
                onClick={addDeliveryTarget}
                disabled={accounts.length === 0}
                className="h-7 px-2 text-xs"
              >
                <Plus className="h-3 w-3 mr-1" />
                {t("cron.addDeliveryTarget")}
              </Button>
            </div>

            {deliveryTargets.length === 0 ? (
              <p className="text-xs text-muted-foreground/60 py-1.5">
                {accounts.length === 0
                  ? t("cron.noDeliveryChannels")
                  : t("cron.noDeliveryTargets")}
              </p>
            ) : (
              <div className="space-y-2">
                {deliveryTargets.map((target, idx) => {
                  const convs = conversationsByAccount[target.accountId] ?? []
                  const selectedConv = convs.find(
                    (c) => c.chatId === target.chatId && (c.threadId ?? null) === (target.threadId ?? null),
                  )
                  return (
                    <div
                      key={idx}
                      className={`flex items-start gap-2 p-2 border rounded-md ${
                        target.stale
                          ? "border-destructive/60 bg-destructive/5"
                          : "border-border bg-muted/20"
                      }`}
                    >
                      <div className="flex-1 space-y-1.5">
                        {target.stale && (
                          <p className="text-[11px] text-destructive flex items-center gap-1">
                            <AlertTriangle className="h-3 w-3" />
                            {t("cron.deliveryTargetStale")}
                          </p>
                        )}
                        <Select
                          value={target.accountId || undefined}
                          onValueChange={(v) => handlePickAccount(idx, v)}
                        >
                          <SelectTrigger className="h-8 text-xs">
                            <SelectValue placeholder={t("cron.selectChannelAccount")} />
                          </SelectTrigger>
                          <SelectContent>
                            {accounts.map((a) => (
                              <SelectItem key={a.id} value={a.id}>
                                <span className="text-xs">
                                  <span className="font-mono text-muted-foreground">
                                    {a.channelId}
                                  </span>
                                  {" · "}
                                  {a.label}
                                </span>
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>

                        <Select
                          value={selectedConv ? String(selectedConv.id) : undefined}
                          onValueChange={(v) => handlePickConversation(idx, v)}
                          disabled={!target.accountId}
                        >
                          <SelectTrigger className="h-8 text-xs">
                            <SelectValue
                              placeholder={
                                !target.accountId
                                  ? t("cron.selectAccountFirst")
                                  : convs.length === 0
                                    ? t("cron.noConversationsYet")
                                    : t("cron.selectConversation")
                              }
                            />
                          </SelectTrigger>
                          <SelectContent>
                            {convs.map((c) => {
                              const name =
                                c.senderName && c.senderName.length > 0
                                  ? c.senderName
                                  : c.chatId
                              return (
                                <SelectItem key={c.id} value={String(c.id)}>
                                  <span className="text-xs">
                                    {name}
                                    <span className="text-muted-foreground ml-1">
                                      ({c.chatType})
                                    </span>
                                  </span>
                                </SelectItem>
                              )
                            })}
                          </SelectContent>
                        </Select>
                      </div>

                      <Button
                        variant="ghost"
                        size="icon"
                        type="button"
                        onClick={() => removeDeliveryTarget(idx)}
                        className="h-7 w-7 shrink-0 text-muted-foreground hover:text-destructive"
                        aria-label={t("cron.removeTarget")}
                      >
                        <X className="h-3.5 w-3.5" />
                      </Button>
                    </div>
                  )
                })}
              </div>
            )}
          </div>

          {/* §8: prefix successful deliveries with the task name (opt-in,
              only meaningful when there are delivery targets) */}
          {deliveryTargets.length > 0 && (
            <div className="flex items-center justify-between">
              <div>
                <label className="text-xs font-medium text-muted-foreground block">
                  {t("cron.prefixDeliveryWithName")}
                </label>
                <p className="text-xs text-muted-foreground/70 mt-0.5">
                  {t("cron.prefixDeliveryWithNameDesc")}
                </p>
              </div>
              <Switch
                checked={prefixDeliveryWithName}
                onCheckedChange={setPrefixDeliveryWithName}
              />
            </div>
          )}

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
