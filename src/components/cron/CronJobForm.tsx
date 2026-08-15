import { useState, useEffect, useMemo } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { NumberInput } from "@/components/ui/number-input"
import { RadioPills } from "@/components/ui/radio-pills"
import { Textarea } from "@/components/ui/textarea"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { X, Plus, Send, FolderOpen, AlertTriangle, GitBranch, Timer } from "lucide-react"
import { AgentSelectDisplay } from "@/components/common/AgentSelectDisplay"
import type {
  CronDeliveryTarget,
  CronJob,
  CronPreflightReport,
  CronSchedule,
  CronUpdateResult,
  CronWorkspaceMode,
} from "./CronJobForm.types"

import type { CronFrequency } from "./CronJobForm.types"
import { parseCronToVisual, buildCronFromVisual, toLocalDatetimeString } from "./cronHelpers"
import CronExpressionBuilder from "./CronExpressionBuilder"
import CronPreflightDialog from "./CronPreflightDialog"
import CronSessionTargetPicker, { type CronSessionTargetOption } from "./CronSessionTargetPicker"
import { DockerSetupHint } from "@/components/settings/DockerSetupHint"
import { useDockerStatus } from "@/hooks/useDockerStatus"
import type { AgentInfo, SandboxMode, SessionMeta, SessionMode } from "@/types/chat"
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

type PendingSave =
  | { operation: "create"; job: object }
  | { operation: "update"; job: CronJob; expectedRevision: number }

// ── Form Props ────────────────────────────────────────────────────

interface CronJobFormProps {
  job?: CronJob | null
  /**
   * Retained task (possibly a tombstone) whose configuration seeds a brand-new
   * draft. Copy-as-new-task, never an edit: no id, no revision, no CAS.
   */
  seedJob?: CronJob | null
  sessionTarget?: { id: string; title?: string | null }
  defaultDate?: Date | null
  defaultProjectId?: string | null
  onSave: () => void
  onCancel: () => void
}

/** Where occurrences run: a fresh isolated chat, or one existing ordinary chat. */
type CronTargetMode = "new" | "existing"

const AUTO_AGENT_VALUE = "__auto__"
const NO_PROJECT_VALUE = "__none__"
/** Sentinel for the permission / sandbox selectors meaning "follow agent default". */
const FOLLOW_MODE_VALUE = "__follow__"
const PERMISSION_MODE_OPTIONS: SessionMode[] = ["default", "smart", "yolo"]
const SANDBOX_MODE_OPTIONS: SandboxMode[] = ["off", "standard", "isolated", "workspace", "trusted"]

export default function CronJobForm({
  job,
  seedJob,
  sessionTarget,
  defaultDate,
  defaultProjectId,
  onSave,
  onCancel,
}: CronJobFormProps) {
  const { t } = useTranslation()
  const isEditing = !!job
  // Configuration source for the initial field values. `job` also drives CAS and
  // edit-only branches; `seedJob` only ever seeds a fresh draft.
  const seed = job ?? seedJob ?? null
  // A target is immutable once a task exists, and pinned when the form was
  // opened from a chat. Otherwise the user picks new-chat vs. an existing chat.
  const lockedSessionId =
    job?.payload.type === "sessionTurn" ? job.payload.sessionId : sessionTarget?.id
  const targetLocked = !!lockedSessionId || isEditing
  const [pickedSession, setPickedSession] = useState<CronSessionTargetOption | null>(() =>
    !targetLocked && seedJob?.payload.type === "sessionTurn"
      ? { id: seedJob.payload.sessionId }
      : null,
  )
  const [targetMode, setTargetMode] = useState<CronTargetMode>(() =>
    !targetLocked && seedJob?.payload.type === "sessionTurn" ? "existing" : "new",
  )
  const sessionTurnId =
    lockedSessionId ?? (targetMode === "existing" ? pickedSession?.id : undefined)
  const isSessionTurn = !!sessionTurnId
  const [baselineJob, setBaselineJob] = useState<CronJob | null>(job ?? null)
  const [expectedRevision, setExpectedRevision] = useState(job?.revision ?? 1)
  const [conflictJob, setConflictJob] = useState<CronJob | null>(null)

  // Form state
  const [name, setName] = useState(seed?.name ?? sessionTarget?.title ?? "")
  const [description, setDescription] = useState(seed?.description ?? "")
  const [scheduleType, setScheduleType] = useState<"at" | "every" | "cron">(
    seed?.schedule.type ?? "cron",
  )
  const [timestamp, setTimestamp] = useState(() => {
    if (seed?.schedule.type === "at" && seed.schedule.timestamp) {
      return toLocalDatetimeString(seed.schedule.timestamp)
    }
    if (defaultDate) {
      return toLocalDatetimeString(defaultDate.toISOString())
    }
    return ""
  })
  // Derive the displayed value + unit from a stored "every" interval, preferring
  // the largest whole unit (so "every 2 hours" loads as 2 + "hour", not 120 +
  // "min"). Without this the unit always reset to "min": a user changing only the
  // unit dropdown on edit would then silently multiply the interval 60× (120 "min"
  // shown → switched to "hour" → 120 hours).
  const initialEvery = (() => {
    const ms =
      seed?.schedule.type === "every"
        ? (seed.schedule.intervalMs ?? seed.schedule.interval_ms)
        : null
    if (!ms) return { value: "60", unit: "min" as const }
    if (ms % 86_400_000 === 0) return { value: String(ms / 86_400_000), unit: "day" as const }
    if (ms % 3_600_000 === 0) return { value: String(ms / 3_600_000), unit: "hour" as const }
    return { value: String(ms / 60_000), unit: "min" as const }
  })()
  const [intervalValue, setIntervalValue] = useState(initialEvery.value)
  const [intervalUnit, setIntervalUnit] = useState<"min" | "hour" | "day">(initialEvery.unit)

  // Visual cron builder state
  const initVisual = useMemo(
    () =>
      parseCronToVisual(
        seed?.schedule.type === "cron" ? (seed.schedule.expression ?? "") : "0 0 9 * * *",
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
    seed?.schedule.type === "cron" ? (seed.schedule.expression ?? "0 0 9 * * *") : "0 0 9 * * *",
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
    if (seed?.schedule.type === "cron") {
      return seed.schedule.timezone || "UTC"
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
    () =>
      timezone && !baseTimezones.includes(timezone) ? [timezone, ...baseTimezones] : baseTimezones,
    [baseTimezones, timezone],
  )

  const [message, setMessage] = useState(seed?.payload.prompt ?? "")
  const [agentId, setAgentId] = useState(
    seed?.payload.type !== "sessionTurn"
      ? (seed?.payload.agentId ?? AUTO_AGENT_VALUE)
      : AUTO_AGENT_VALUE,
  )
  const [projectId, setProjectId] = useState(
    seed ? (seed.projectId ?? NO_PROJECT_VALUE) : (defaultProjectId ?? NO_PROJECT_VALUE),
  )
  const [workspaceMode, setWorkspaceMode] = useState<CronWorkspaceMode>(
    seed?.workspacePolicy?.mode ?? "project",
  )
  const [workspaceBaseRef, setWorkspaceBaseRef] = useState(seed?.workspacePolicy?.baseRef ?? "")
  const [maxFailures, setMaxFailures] = useState(String(seed?.maxFailures ?? 5))
  const [notifyOnComplete, setNotifyOnComplete] = useState(seed?.notifyOnComplete ?? true)
  const [prefixDeliveryWithName, setPrefixDeliveryWithName] = useState(
    seed?.prefixDeliveryWithName ?? false,
  )
  // C19: per-job timeout override; blank string = use the global default.
  const [jobTimeoutSecs, setJobTimeoutSecs] = useState(
    seed?.jobTimeoutSecs != null ? String(seed.jobTimeoutSecs) : "",
  )
  // Per-job permission / sandbox overrides; FOLLOW sentinel = follow agent default.
  const [permissionModeOverride, setPermissionModeOverride] = useState<string>(
    seed?.permissionModeOverride ?? FOLLOW_MODE_VALUE,
  )
  const [sandboxModeOverride, setSandboxModeOverride] = useState<string>(
    seed?.sandboxModeOverride ?? FOLLOW_MODE_VALUE,
  )
  const {
    status: dockerStatus,
    checking: dockerChecking,
    ready: dockerReady,
    refresh: checkDocker,
  } = useDockerStatus()
  // The selected sandbox needs Docker iff an explicit non-off mode is chosen.
  // "Follow agent" can't be resolved here without the agent's effective default,
  // so the hint only shows for an explicit non-off pick (conservative — runtime
  // still fail-closes if Docker is down at fire time).
  const sandboxNeedsDocker =
    !isSessionTurn && sandboxModeOverride !== FOLLOW_MODE_VALUE && sandboxModeOverride !== "off"
  useEffect(() => {
    if (sandboxNeedsDocker) void checkDocker()
  }, [sandboxNeedsDocker, checkDocker])
  const [deliveryTargets, setDeliveryTargets] = useState<CronDeliveryTarget[]>(
    () => seed?.deliveryTargets?.map((t) => ({ ...t })) ?? [],
  )
  const [accounts, setAccounts] = useState<ChannelAccountConfig[]>([])
  const [projects, setProjects] = useState<ProjectMeta[]>([])
  const [conversationsByAccount, setConversationsByAccount] = useState<
    Record<string, ChannelConversationDto[]>
  >({})
  const [agents, setAgents] = useState<AgentInfo[]>([])
  const [sessionTurnTitle, setSessionTurnTitle] = useState(sessionTarget?.title ?? null)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState("")
  const [pendingSave, setPendingSave] = useState<PendingSave | null>(null)
  const [preflight, setPreflight] = useState<CronPreflightReport | null>(null)
  const selectedAgent = agentId === AUTO_AGENT_VALUE ? null : agents.find((a) => a.id === agentId)
  const selectedProject =
    projectId === NO_PROJECT_VALUE ? null : projects.find((p) => p.id === projectId)
  const isMissingProject = projectId !== NO_PROJECT_VALUE && !selectedProject

  const loadConflictVersion = () => {
    if (!conflictJob) return
    const latest = conflictJob
    setBaselineJob(latest)
    setExpectedRevision(latest.revision)
    setName(latest.name)
    setDescription(latest.description ?? "")
    setScheduleType(latest.schedule.type)
    if (latest.schedule.type === "at") {
      setTimestamp(
        latest.schedule.timestamp ? toLocalDatetimeString(latest.schedule.timestamp) : "",
      )
    } else if (latest.schedule.type === "every") {
      const ms = latest.schedule.intervalMs ?? latest.schedule.interval_ms ?? 60_000
      if (ms % 86_400_000 === 0) {
        setIntervalValue(String(ms / 86_400_000))
        setIntervalUnit("day")
      } else if (ms % 3_600_000 === 0) {
        setIntervalValue(String(ms / 3_600_000))
        setIntervalUnit("hour")
      } else {
        setIntervalValue(String(ms / 60_000))
        setIntervalUnit("min")
      }
    } else {
      const visual = parseCronToVisual(latest.schedule.expression ?? "0 0 9 * * *")
      setCronFreq(visual.freq)
      setCronHour(visual.hour)
      setCronMinute(visual.minute)
      setCronWeekdays(visual.weekdays)
      setCronMonthDay(visual.monthDay)
      setCronRawExpr(latest.schedule.expression ?? "0 0 9 * * *")
      setTimezone(latest.schedule.timezone || "UTC")
    }
    setMessage(latest.payload.prompt)
    setAgentId(
      latest.payload.type !== "sessionTurn"
        ? (latest.payload.agentId ?? AUTO_AGENT_VALUE)
        : AUTO_AGENT_VALUE,
    )
    setProjectId(latest.projectId ?? NO_PROJECT_VALUE)
    setWorkspaceMode(latest.workspacePolicy?.mode ?? "project")
    setWorkspaceBaseRef(latest.workspacePolicy?.baseRef ?? "")
    setMaxFailures(String(latest.maxFailures))
    setNotifyOnComplete(latest.notifyOnComplete)
    setPrefixDeliveryWithName(latest.prefixDeliveryWithName ?? false)
    setJobTimeoutSecs(latest.jobTimeoutSecs != null ? String(latest.jobTimeoutSecs) : "")
    setPermissionModeOverride(latest.permissionModeOverride ?? FOLLOW_MODE_VALUE)
    setSandboxModeOverride(latest.sandboxModeOverride ?? FOLLOW_MODE_VALUE)
    setDeliveryTargets(latest.deliveryTargets.map((target) => ({ ...target })))
    setConflictJob(null)
    setError("")
  }

  useEffect(() => {
    if (!isSessionTurn) {
      getTransport()
        .call<AgentInfo[]>("list_agents")
        .then(setAgents)
        .catch(() => {})

      getTransport()
        .call<ProjectMeta[]>("list_projects_cmd", { includeArchived: true })
        .then((list) => setProjects(Array.isArray(list) ? list : []))
        .catch(() => {})
    }

    getTransport()
      .call<ChannelAccountConfig[]>("channel_list_accounts")
      .then((list) => setAccounts(list.filter((a) => a.enabled)))
      .catch(() => {})
  }, [isSessionTurn])

  // Resolve the target chat's title for display. The picker already knows it;
  // a target restored from a job payload or a chat handoff does not.
  const knownTargetTitle = sessionTarget?.title ?? pickedSession?.title ?? null
  useEffect(() => {
    if (!sessionTurnId) return
    if (knownTargetTitle) {
      setSessionTurnTitle(knownTargetTitle)
      return
    }
    let cancelled = false
    getTransport()
      .call<SessionMeta | null>("get_session_cmd", { sessionId: sessionTurnId })
      .then((session) => {
        if (!cancelled) setSessionTurnTitle(session?.title ?? null)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [knownTargetTitle, sessionTurnId])

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
    if (!targetLocked && targetMode === "existing" && !pickedSession) {
      setError(t("cron.errorSessionTargetRequired"))
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
    const validTargets = deliveryTargets.filter((t) => t.channelId && t.accountId && t.chatId)
    // FOLLOW sentinel → null (follow agent default); else the explicit mode.
    const resolvedPermissionMode: SessionMode | null =
      permissionModeOverride === FOLLOW_MODE_VALUE ? null : (permissionModeOverride as SessionMode)
    const resolvedSandboxMode: SandboxMode | null =
      sandboxModeOverride === FOLLOW_MODE_VALUE ? null : (sandboxModeOverride as SandboxMode)
    const parseJobTimeoutSecs = () => {
      const raw = jobTimeoutSecs.trim()
      if (!raw) return null
      const parsed = Number(raw)
      return Number.isFinite(parsed) ? Math.floor(parsed) : null
    }

    try {
      const candidate = {
        name: name.trim(),
        description: description.trim() || null,
        projectId: isSessionTurn || projectId === NO_PROJECT_VALUE ? null : projectId,
        workspacePolicy: {
          mode: isSessionTurn ? "project" : workspaceMode,
          baseRef:
            isSessionTurn || workspaceMode === "project" ? null : workspaceBaseRef.trim() || null,
        },
        schedule: buildSchedule(),
        payload: isSessionTurn
          ? { type: "sessionTurn" as const, sessionId: sessionTurnId!, prompt: message.trim() }
          : {
              type: "agentTurn" as const,
              prompt: message.trim(),
              agentId: agentId === AUTO_AGENT_VALUE ? null : agentId,
            },
        maxFailures: parseInt(maxFailures) || 5,
        notifyOnComplete,
        deliveryTargets: validTargets,
        prefixDeliveryWithName,
        jobTimeoutSecs: parseJobTimeoutSecs(),
        permissionModeOverride: isSessionTurn ? null : resolvedPermissionMode,
        sandboxModeOverride: isSessionTurn ? null : resolvedSandboxMode,
      }
      const pending: PendingSave =
        isEditing && job
          ? {
              operation: "update",
              job: { ...(baselineJob ?? job), ...candidate },
              expectedRevision,
            }
          : { operation: "create", job: candidate }
      const report = await getTransport().call<CronPreflightReport>("cron_preflight", {
        request: pending,
      })
      if (report.issues.some((issue) => issue.code === "revision_conflict")) {
        setPreflight(null)
        setPendingSave(null)
        const current = job
          ? await getTransport()
              .call<CronJob | null>("cron_get_job", { id: job.id })
              .catch(() => null)
          : null
        setConflictJob(current)
        setError(t("cron.revisionConflict"))
        return
      }
      setPendingSave(pending)
      setPreflight(report)
    } catch (e: unknown) {
      setError(String(e))
    } finally {
      setSaving(false)
    }
  }

  async function confirmSave() {
    if (!pendingSave || !preflight?.canProceed) return
    setSaving(true)
    setError("")
    try {
      if (pendingSave.operation === "update") {
        const result = await getTransport().call<CronUpdateResult>("cron_update_job", {
          job: pendingSave.job,
          expectedRevision: pendingSave.expectedRevision,
        })
        if (!result.updated) {
          const current = result.currentJob ?? null
          setConflictJob(current)
          setError(t("cron.revisionConflict"))
          setPreflight(null)
          setPendingSave(null)
          return
        }
      } else {
        await getTransport().call("cron_create_job", { job: pendingSave.job })
      }
      onSave()
    } catch (e: unknown) {
      setError(String(e))
      setPreflight(null)
      setPendingSave(null)
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
          baselineJob?.schedule.type === "every" &&
          (baselineJob.schedule.intervalMs ?? baselineJob.schedule.interval_ms) === intervalMs
        return {
          type: "every",
          intervalMs,
          startAt: preserveStartAt
            ? (baselineJob.schedule.startAt ?? baselineJob.schedule.start_at ?? null)
            : undefined,
        }
      }
      case "cron":
        return { type: "cron", expression: cronExpression, timezone: timezone || null }
    }
  }

  return (
    <div className="fixed inset-0 z-50 bg-black/50 flex items-center justify-center p-4">
      <div className="bg-card border border-border rounded-xl shadow-xl w-full max-w-3xl max-h-[90vh] overflow-y-auto">
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-border">
          <h3 className="text-base font-medium">
            {isEditing
              ? t("cron.editJob")
              : seedJob
                ? t("cron.copyAsNewTask")
                : lockedSessionId
                  ? t("cron.scheduleSession")
                  : t("cron.newJob")}
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
                <NumberInput
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
                <p className="text-[11px] text-muted-foreground mt-1">{t("cron.timezoneHint")}</p>
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

          {/* Conversation target — immutable once the task exists, so it is only
              editable while creating a task outside a chat handoff. */}
          {!targetLocked && (
            <div className="space-y-2 rounded-md border border-border/50 p-3">
              <label className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
                <Timer className="h-3.5 w-3.5" />
                {t("cron.conversationTarget")}
              </label>
              <RadioPills<CronTargetMode>
                value={targetMode}
                onChange={setTargetMode}
                cols="grid-cols-2"
                ariaLabel={t("cron.conversationTarget")}
                options={[
                  { value: "new", label: t("cron.conversationTargetNew") },
                  { value: "existing", label: t("cron.conversationTargetExisting") },
                ]}
              />
              {targetMode === "existing" && (
                <>
                  <CronSessionTargetPicker value={pickedSession} onChange={setPickedSession} />
                  <p className="text-[11px] leading-4 text-muted-foreground">
                    {t("cron.sessionTurnLiveHint")}
                  </p>
                </>
              )}
            </div>
          )}

          {isSessionTurn ? (
            targetLocked && (
              <div className="rounded-md border border-border/50 bg-muted/20 p-3">
                <p className="flex items-center gap-1.5 text-xs font-medium">
                  <Timer className="h-3.5 w-3.5 text-primary" />
                  {t("cron.sessionTurnTarget")}: {sessionTurnTitle || sessionTurnId}
                </p>
                <p className="mt-1 text-[11px] leading-4 text-muted-foreground">
                  {t("cron.sessionTurnLiveHint")}
                </p>
              </div>
            )
          ) : (
            <>
              {/* Project */}
              <div>
                <label className="text-xs font-medium text-muted-foreground mb-1 flex items-center gap-1.5">
                  <FolderOpen className="h-3 w-3" />
                  {t("cron.project")}
                </label>
                <Select
                  value={projectId}
                  onValueChange={(value) => {
                    setProjectId(value)
                    if (value === NO_PROJECT_VALUE) setWorkspaceMode("project")
                  }}
                >
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

              {/* Execution workspace */}
              <div className="space-y-2 rounded-md border border-border/50 p-3">
                <label className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
                  <GitBranch className="h-3.5 w-3.5" />
                  {t("workspace.environment.worktree")}
                </label>
                <RadioPills<CronWorkspaceMode>
                  value={workspaceMode}
                  onChange={setWorkspaceMode}
                  ariaLabel={t("workspace.environment.worktree")}
                  options={(["project", "fresh", "persistent"] as const).map((mode) => ({
                    value: mode,
                    label:
                      mode === "project"
                        ? t("cron.project")
                        : t(
                            mode === "fresh"
                              ? "chat.projectRuntime.worktree"
                              : "cron.workspaceModePersistent",
                          ),
                    disabled: mode !== "project" && projectId === NO_PROJECT_VALUE,
                  }))}
                />
                <p className="text-[11px] text-muted-foreground">{t("cron.workspaceModeHint")}</p>
                {workspaceMode !== "project" && (
                  <div>
                    <label className="mb-1 block text-xs font-medium text-muted-foreground">
                      {t("workspace.git.prBase")}
                    </label>
                    <Input
                      value={workspaceBaseRef}
                      onChange={(event) => setWorkspaceBaseRef(event.target.value)}
                      placeholder="HEAD"
                    />
                  </div>
                )}
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

              {/* Permission + sandbox overrides (per-job; default = follow agent) */}
              <div className="space-y-2 rounded-md border border-border/50 p-3">
                <div>
                  <label className="text-xs font-medium text-muted-foreground mb-1 block">
                    {t("cron.permissionMode")}
                  </label>
                  <Select value={permissionModeOverride} onValueChange={setPermissionModeOverride}>
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value={FOLLOW_MODE_VALUE}>{t("cron.followAgentMode")}</SelectItem>
                      {PERMISSION_MODE_OPTIONS.map((m) => (
                        <SelectItem key={m} value={m}>
                          {t(`cron.permissionMode_${m}`)}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div>
                  <label className="text-xs font-medium text-muted-foreground mb-1 block">
                    {t("cron.sandboxMode")}
                  </label>
                  <Select value={sandboxModeOverride} onValueChange={setSandboxModeOverride}>
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value={FOLLOW_MODE_VALUE}>{t("cron.followAgentMode")}</SelectItem>
                      {SANDBOX_MODE_OPTIONS.map((m) => (
                        <SelectItem key={m} value={m}>
                          {t(`chat.sandboxMode.${m}.label`, { defaultValue: m })}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <p className="text-[10px] text-muted-foreground">
                  {t("cron.permissionSandboxHint")}
                </p>
                {sandboxNeedsDocker &&
                  (!dockerReady ||
                    (dockerStatus?.isolatedModeOnly && sandboxModeOverride !== "isolated")) && (
                    <DockerSetupHint
                      status={dockerStatus}
                      checking={dockerChecking}
                      onRefresh={checkDocker}
                      sandboxMode={sandboxModeOverride}
                      title={t("chat.sandboxMode.setupTitle", {
                        defaultValue: "配置 Docker 后启用沙箱",
                      })}
                    />
                  )}
                {permissionModeOverride === "yolo" && sandboxModeOverride === "off" && (
                  <div className="flex items-start gap-1.5 rounded-md border border-destructive/40 bg-destructive/10 p-2 text-[11px] text-destructive">
                    <AlertTriangle className="h-3.5 w-3.5 mt-0.5 shrink-0" />
                    <span>{t("cron.unsandboxedYoloWarning")}</span>
                  </div>
                )}
              </div>
            </>
          )}

          {/* Max Failures */}
          <div>
            <label className="text-xs font-medium text-muted-foreground mb-1 block">
              {t("cron.maxFailures")}
            </label>
            <NumberInput
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
            <NumberInput
              min="0"
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
          {error && (
            <div className="space-y-2 text-xs text-red-500">
              <p>{error}</p>
              {conflictJob && (
                <div className="flex gap-2">
                  <Button type="button" variant="outline" size="sm" onClick={loadConflictVersion}>
                    {t("cron.reloadLatest")}
                  </Button>
                </div>
              )}
            </div>
          )}
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
      <CronPreflightDialog
        report={preflight}
        busy={saving}
        confirmLabel={isEditing ? t("common.save") : t("cron.create")}
        onClose={() => {
          setPreflight(null)
          setPendingSave(null)
        }}
        onConfirm={() => void confirmSave()}
        onRetry={() => void handleSave()}
      />
    </div>
  )
}
