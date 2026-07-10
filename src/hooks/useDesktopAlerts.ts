import { useEffect, useRef } from "react"
import { useTranslation } from "react-i18next"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload } from "@/lib/transport"
import { MCP_EVENTS } from "@/lib/mcp"
import { getCachedConfig, notifyIfBackground } from "@/lib/notifications"
import { logger } from "@/lib/logger"
import { LOCAL_MODEL_JOB_EVENTS } from "@/types/local-model-jobs"

// Background-job terminal statuses worth a "跑完叫我" notification — skip
// user-cancelled / restart-interrupted (not noteworthy outcomes).
const NOTIFIABLE_JOB_STATUSES = new Set(["completed", "failed", "timed_out"])

// Below this, skip the desktop escalation for knowledge scan / import
// completions (the in-app toast + activity panel still reflect it) — binding
// a small space or importing a couple of sources finishes fast enough that a
// system notification is just noise, not a "you can stop watching" signal.
const MIN_JOB_DURATION_FOR_NOTIFICATION_SECS = 8

// Truncate user-visible strings (commands, questions) so notifications
// don't blow past Notification Center's character limit.
function truncate(s: string, max = 80): string {
  if (!s) return ""
  return s.length > max ? `${s.slice(0, max - 1)}…` : s
}

function textPreview(value: unknown): string {
  if (typeof value === "string") return value
  if (value && typeof value === "object") {
    const record = value as { fallback?: unknown; key?: unknown }
    if (typeof record.fallback === "string") return record.fallback
    if (typeof record.key === "string") return record.key
  }
  return ""
}

// Leading-edge cooldown for channel auth failures. When several IM
// accounts fail at the same instant (e.g. system clock jump invalidates
// every bot token at once, or post-resume DNS hasn't recovered for any
// of them), the watchdog emits one event per account back-to-back. We
// surface only the first inside this window — the user just needs to
// know "go check your IM channels," not get N back-to-back popups.
const CHANNEL_AUTH_COOLDOWN_MS = 1500

export function useDesktopAlerts() {
  const { t } = useTranslation()
  // Keep `t` in a ref so the listener effect can re-read the current
  // translation function without re-subscribing on every i18n change.
  const tRef = useRef(t)
  useEffect(() => {
    tRef.current = t
  }, [t])
  // Knowledge source import's `job:created`/`job:completed` payloads carry no
  // timestamp (unlike `local_model_job:*`, which is the full snapshot and
  // already has `createdAt`) — track start times locally so the duration gate
  // above has something to compare against.
  const importJobStartedAtRef = useRef<Map<string, number>>(new Map())

  useEffect(() => {
    const transport = getTransport()
    let lastChannelAuthAt = 0

    function bindAlert<T>(
      event: string,
      source: string,
      build: (p: T) => { title: string; body: string } | null,
    ): () => void {
      return transport.listen(event, (raw) => {
        try {
          const parsed = parsePayload<T>(raw)
          if (!parsed) return
          const result = build(parsed)
          if (result) notifyIfBackground(result.title, result.body)
        } catch (e) {
          logger.error("ui", source, `Bad ${event} payload`, e)
        }
      })
    }

    const offApproval = bindAlert<{ command?: string }>(
      "approval_required",
      "useDesktopAlerts::approval",
      (req) => {
        const tx = tRef.current
        const body = truncate(req?.command ?? "")
        return {
          title: tx("notification.approvalRequired"),
          body: body || tx("notification.approvalRequiredFallback"),
        }
      },
    )

    const offApprovalTimedOut = bindAlert<{
      timeout_secs?: number
      timeout_action?: "deny" | "proceed"
    }>(
      "approval_timed_out",
      "useDesktopAlerts::approval_timeout",
      (event) => {
        const tx = tRef.current
        const bodyKey =
          event?.timeout_action === "proceed"
            ? "notification.approvalTimedOut_proceeded"
            : "notification.approvalTimedOut_denied"
        return {
          title: tx("notification.approvalTimedOut"),
          body: tx(bodyKey, {
            seconds: event?.timeout_secs ?? 0,
          }),
        }
      },
    )

    const offAskUser = bindAlert<{ questions?: Array<{ text?: unknown }> }>(
      "ask_user_request",
      "useDesktopAlerts::ask_user",
      (group) => {
        const tx = tRef.current
        const firstQ = truncate(textPreview(group?.questions?.[0]?.text))
        return {
          title: tx("notification.askUserRequired"),
          body: firstQ || tx("notification.askUserRequiredFallback"),
        }
      },
    )

    const offAskUserTimedOut = bindAlert<{
      timeoutSecs?: number
      usedDefaultValues?: boolean
      questionPreview?: string
    }>(
      "ask_user_timed_out",
      "useDesktopAlerts::ask_user_timeout",
      (event) => {
        const tx = tRef.current
        const preview = truncate(event?.questionPreview ?? "")
        const body = tx(
          event?.usedDefaultValues
            ? "notification.askUserTimedOutDefaults"
            : "notification.askUserTimedOutNoDefaults",
          { seconds: event?.timeoutSecs ?? 0 },
        )
        return {
          title: tx("notification.askUserTimedOut"),
          body: preview ? `${preview} — ${body}` : body,
        }
      },
    )

    const offMcpAuth = bindAlert<{ name?: string }>(
      MCP_EVENTS.AUTH_REQUIRED,
      "useDesktopAlerts::mcp_auth",
      (ev) => {
        const tx = tRef.current
        const name = ev?.name ?? ""
        return {
          title: tx("notification.mcpAuthRequired"),
          body: name
            ? tx("notification.mcpAuthRequiredBody", { name })
            : tx("notification.mcpAuthRequiredFallback"),
        }
      },
    )

    const offChannelAuth = bindAlert<{ label?: string; channelId?: string }>(
      "channel:auth_failed",
      "useDesktopAlerts::channel_auth",
      (ev) => {
        const now = Date.now()
        if (now - lastChannelAuthAt < CHANNEL_AUTH_COOLDOWN_MS) return null
        lastChannelAuthAt = now
        const tx = tRef.current
        const label = ev?.label || ev?.channelId || ""
        return {
          title: tx("notification.channelAuthFailed"),
          body: label
            ? tx("notification.channelAuthFailedBody", { label })
            : tx("notification.channelAuthFailedFallback"),
        }
      },
    )

    // Record knowledge import job start times for the duration gate below.
    // Not routed through `bindAlert` — it never produces a notification by
    // itself.
    const offImportJobCreated = transport.listen("job:created", (raw) => {
      const ev = parsePayload<{ job_id?: string; tool?: string }>(raw)
      if (ev?.job_id && ev.tool === "knowledge_source_import") {
        importJobStartedAtRef.current.set(ev.job_id, Date.now())
      }
    })

    // R4: "跑完叫我" — a background job (tool / group, R3 `job:*`) finishing in
    // the background fires a desktop notification, gated by the dedicated
    // `notifyOnBackgroundJobComplete` toggle. Subagent jobs ride `subagent:*` and
    // are out of scope here. Background-only via `notifyIfBackground` (the user
    // already sees the panel/badge when the window is up front).
    const offJobCompleted = bindAlert<{
      job_id?: string
      tool?: string
      status?: string
      kind?: string
      imported_count?: number
      duplicate_count?: number
      failed_count?: number
    }>("job:completed", "useDesktopAlerts::job_completed", (ev) => {
      if (getCachedConfig()?.notifyOnBackgroundJobComplete === false) return null
      const status = ev?.status ?? ""
      if (!NOTIFIABLE_JOB_STATUSES.has(status)) return null
      const tx = tRef.current

      if (ev?.tool === "knowledge_source_import") {
        const jobId = ev.job_id
        const startedAt = jobId ? importJobStartedAtRef.current.get(jobId) : undefined
        if (jobId) importJobStartedAtRef.current.delete(jobId)
        // Unknown start time (this hook mounted after the import began) →
        // don't suppress; better to over-notify than silently drop the only
        // signal the user might get for a job they never saw start.
        const durationSecs = startedAt ? (Date.now() - startedAt) / 1000 : Infinity
        if (durationSecs < MIN_JOB_DURATION_FOR_NOTIFICATION_SECS) return null
        const imported = ev.imported_count ?? 0
        const duplicate = ev.duplicate_count ?? 0
        const failedCount = ev.failed_count ?? 0
        const body =
          failedCount > 0
            ? tx("knowledge.sources.importRunPartial", { imported, duplicate, failed: failedCount })
            : duplicate > 0
              ? tx("knowledge.sources.importRunDeduped", { imported, duplicate })
              : tx("knowledge.sources.importedCount", { count: imported })
        return {
          title:
            failedCount > 0
              ? tx("notification.knowledgeImportFailed", "知识空间导入部分失败")
              : tx("notification.knowledgeImportComplete", "知识空间导入完成"),
          body,
        }
      }

      const failed = status !== "completed"
      // A Group's `tool` is the internal id "subagent:batch" — show a friendly
      // localized label instead; tool jobs surface their real tool name.
      const body =
        ev?.kind === "group"
          ? tx("backgroundJobs.kindGroup", "任务组")
          : truncate(ev?.tool ?? "") ||
            tx("notification.backgroundJobFallback", "一个后台任务已结束")
      return {
        title: failed
          ? tx("notification.backgroundJobFailed", "后台任务失败")
          : tx("notification.backgroundJobComplete", "后台任务完成"),
        body,
      }
    })

    // Knowledge space reindex/re-embed completion (bind a new space,
    // per-space Reindex, or the settings-page "rebuild everything" — all
    // `knowledge_reembed`, see `reembed.rs`). `local_model_job:*` was
    // previously not wired into desktop alerts at all, so binding a large
    // vault and tabbing away gave no signal it had finished.
    const offKnowledgeReembedCompleted = bindAlert<{
      kind?: string
      status?: string
      createdAt?: number
      completedAt?: number
      error?: string
      resultJson?: { reindexed?: number; failedFiles?: number; kbCount?: number } | null
    }>(LOCAL_MODEL_JOB_EVENTS.completed, "useDesktopAlerts::knowledge_reembed", (ev) => {
      if (ev?.kind !== "knowledge_reembed") return null
      if (getCachedConfig()?.notifyOnBackgroundJobComplete === false) return null
      const status = ev?.status ?? ""
      if (!NOTIFIABLE_JOB_STATUSES.has(status)) return null
      const durationSecs = (ev.completedAt ?? 0) - (ev.createdAt ?? 0)
      if (durationSecs < MIN_JOB_DURATION_FOR_NOTIFICATION_SECS) return null
      const tx = tRef.current
      const failed = status !== "completed"
      const failedFiles = ev.resultJson?.failedFiles ?? 0
      const body = failed
        ? truncate(ev.error ?? "") || tx("notification.backgroundJobFallback", "一个后台任务已结束")
        : failedFiles > 0
          ? tx("knowledge.jobs.resultPartial", {
              notes: ev.resultJson?.reindexed ?? 0,
              failed: failedFiles,
              defaultValue: "Reindexed {{notes}} notes · {{failed}} skipped",
            })
          : tx("knowledge.jobs.resultOk", {
              notes: ev.resultJson?.reindexed ?? 0,
              defaultValue: "Reindexed {{notes}} notes",
            })
      return {
        title: failed
          ? tx("notification.knowledgeScanFailed", "知识空间扫描失败")
          : tx("notification.knowledgeScanComplete", "知识空间扫描完成"),
        body,
      }
    })

    return () => {
      offApproval()
      offApprovalTimedOut()
      offAskUser()
      offAskUserTimedOut()
      offMcpAuth()
      offChannelAuth()
      offImportJobCreated()
      offJobCompleted()
      offKnowledgeReembedCompleted()
    }
  }, [])
}
