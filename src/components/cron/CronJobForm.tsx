import { useEffect, useState } from "react"
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
import { X, Bot, Plus, Send } from "lucide-react"
import type { CronDeliveryTarget, CronJob, CronSchedule } from "./CronJobForm.types"

import type { CronFrequency } from "./CronJobForm.types"
import { parseCronToVisual, buildCronFromVisual, toLocalDatetimeString } from "./cronHelpers"
import CronExpressionBuilder from "./CronExpressionBuilder"
import type { AgentInfo } from "@/types/chat"
import type { ChannelAccountConfig } from "@/components/settings/channel-panel/types"

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
    const intervalMs =
      job?.schedule.type === "every" ? (job.schedule.intervalMs ?? job.schedule.interval_ms) : null
    if (intervalMs) {
      return String(intervalMs / 60000)
    }
    return "60"
  })
  const [intervalUnit, setIntervalUnit] = useState<"min" | "hour" | "day">("min")

  // Visual cron builder state
  const initVisual = parseCronToVisual(
    job?.schedule.type === "cron" ? (job.schedule.expression ?? "") : "0 0 9 * * *",
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
  const cronExpression = buildCronFromVisual(
    cronFreq,
    cronHour,
    cronMinute,
    cronWeekdays,
    cronMonthDay,
    cronRawExpr,
  )

  const [message, setMessage] = useState(job?.payload.prompt ?? "")
  const [agentId, setAgentId] = useState(job?.payload.agentId ?? "default")
  const [maxFailures, setMaxFailures] = useState(String(job?.maxFailures ?? 5))
  const [notifyOnComplete, setNotifyOnComplete] = useState(job?.notifyOnComplete ?? true)
  const [deliveryTargets, setDeliveryTargets] = useState<CronDeliveryTarget[]>(
    () => job?.deliveryTargets?.map((t) => ({ ...t })) ?? [],
  )
  const [accounts, setAccounts] = useState<ChannelAccountConfig[]>([])
  const [conversationsByAccount, setConversationsByAccount] = useState<
    Record<string, ChannelConversationDto[]>
  >({})
  const [agents, setAgents] = useState<AgentInfo[]>([])
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState("")

  useEffect(() => {
    getTransport()
      .call<AgentInfo[]>("list_agents")
      .then(setAgents)
      .catch(() => {})

    getTransport()
      .call<ChannelAccountConfig[]>("channel_list_accounts")
      .then((list) => setAccounts(list.filter((a) => a.enabled)))
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
      const list = await getTransport().call<ChannelConversationDto[]>("channel_list_sessions", {
        channelId,
        accountId,
      })
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

    setSaving(true)
    setError("")

    // Only persist fully-configured targets (skip rows still awaiting a chat pick).
    const validTargets = deliveryTargets.filter((t) => t.channelId && t.accountId && t.chatId)

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
          deliveryTargets: validTargets,
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
            deliveryTargets: validTargets,
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
        const intervalMs = Math.max(60000, num * multiplier)
        const preserveStartAt =
          job?.schedule.type === "every" &&
          (job.schedule.intervalMs ?? job.schedule.interval_ms) === intervalMs
        return {
          type: "every",
          intervalMs,
          startAt: preserveStartAt
            ? (job.schedule.startAt ?? job.schedule.start_at ?? null)
            : undefined,
        }
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
                            src={getTransport().resolveAssetUrl(a.avatar) ?? a.avatar}
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
                {accounts.length === 0 ? t("cron.noDeliveryChannels") : t("cron.noDeliveryTargets")}
              </p>
            ) : (
              <div className="space-y-2">
                {deliveryTargets.map((target, idx) => {
                  const convs = conversationsByAccount[target.accountId] ?? []
                  const selectedConv = convs.find(
                    (c) =>
                      c.chatId === target.chatId &&
                      (c.threadId ?? null) === (target.threadId ?? null),
                  )
                  return (
                    <div
                      key={idx}
                      className="flex items-start gap-2 p-2 border border-border rounded-md bg-muted/20"
                    >
                      <div className="flex-1 space-y-1.5">
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
                                c.senderName && c.senderName.length > 0 ? c.senderName : c.chatId
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
