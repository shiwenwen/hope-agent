import type {
  CronSchedule,
  CronFrequency,
  CronDeliveryTarget,
  CronPayloadType,
  CronJob,
} from "./CronJobForm.types"

const LOOP_TITLE_PREFIX = /^\s*\[Loop\]\s*/i

/**
 * Remove the legacy internal title prefix only after the structured payload
 * discriminator has established that this really is a Loop. Ordinary jobs
 * named `[Loop] ...` remain untouched and never receive a Loop badge.
 */
export function cronDisplayTitle(title: string, payloadType?: CronPayloadType | null): string {
  if (payloadType !== "sessionLoop") return title
  return title.replace(LOOP_TITLE_PREFIX, "") || title
}

/** User-visible state follows the Loop control plane when one owns the job. */
export function cronDisplayStatus(job: CronJob): CronJob["status"] {
  if (job.payload.type !== "sessionLoop" || !job.loopState) return job.status
  switch (job.loopState) {
    case "active":
      return "active"
    case "paused":
      return "paused"
    case "blocked":
      return "disabled"
    case "completed":
    case "cancelled":
      return "completed"
  }
}

/**
 * Human-readable label for a delivery target. Uses the cached `label` computed
 * when the target was picked (e.g. `telegram / 张三`); falls back to the raw
 * `channelId / chatId` for targets created without a label (e.g. via the model
 * tool). No extra data fetch needed.
 */
export function deliveryTargetLabel(target: CronDeliveryTarget): string {
  const cached = target.label?.trim()
  if (cached) return cached
  return `${target.channelId} / ${target.chatId}`
}

/** Tailwind text color for a run's delivery status (delivered/partial/failed). */
export function deliveryStatusColor(status: string): string {
  return status === "delivered"
    ? "text-emerald-500"
    : status === "partial"
      ? "text-amber-500"
      : "text-red-500"
}

export const WEEKDAY_KEYS = [
  "weekMon",
  "weekTue",
  "weekWed",
  "weekThu",
  "weekFri",
  "weekSat",
  "weekSun",
] as const

export const WEEKDAY_CRON = [1, 2, 3, 4, 5, 6, 0] // cron weekday values (Mon=1 .. Sun=0)

/** Parse an existing cron expression into visual-builder state (best effort). */
export function parseCronToVisual(expr: string): {
  freq: CronFrequency
  hour: string
  minute: string
  weekdays: boolean[]
  monthDay: string
} {
  const defaults = {
    freq: "daily" as CronFrequency,
    hour: "09",
    minute: "00",
    weekdays: Array(7).fill(false) as boolean[],
    monthDay: "1",
  }
  if (!expr) return defaults

  // cron crate uses 7 fields: sec min hour day month weekday [year]
  const parts = expr.trim().split(/\s+/)
  if (parts.length < 6) return { ...defaults, freq: "custom" }

  const [, min, hour, day, , weekday] = parts

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
export function buildCronFromVisual(
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
      const selected = weekdays.map((on, i) => (on ? WEEKDAY_CRON[i] : -1)).filter((v) => v >= 0)
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

export function toLocalDatetimeString(isoString: string): string {
  try {
    const d = new Date(isoString)
    const pad = (n: number) => String(n).padStart(2, "0")
    return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`
  } catch {
    return ""
  }
}

// Single source of truth for job-status presentation. `statusColor` and
// `statusLabel` both read this map so a new status can't get a dot color but a
// missing tooltip (or vice versa). Unknown statuses fall back to gray / raw value.
const STATUS_META: Record<string, { color: string; labelKey: string }> = {
  active: { color: "bg-blue-500", labelKey: "cron.active" },
  paused: { color: "bg-amber-500", labelKey: "cron.paused" },
  disabled: { color: "bg-red-500", labelKey: "cron.disabled" },
  completed: { color: "bg-emerald-500", labelKey: "cron.completed" },
  missed: { color: "bg-orange-500", labelKey: "cron.missed" },
}

/** Localized label for a job status, paired with `statusColor` for the status
 *  dot tooltip. Unknown statuses (gray dot) fall back to the raw value. */
export function statusLabel(status: string, t: (key: string) => string): string {
  const meta = STATUS_META[status]
  return meta ? t(meta.labelKey) : status
}

export function statusColor(status: string): string {
  return STATUS_META[status]?.color ?? "bg-gray-400"
}

/**
 * Dot color for a calendar occurrence given its matched run-log status (if any)
 * and the owning job's status. C21: `empty` / `cancelled` / `running` runs get
 * neutral / in-progress colors instead of falling through to the job's status
 * color (which made an already-run occurrence indistinguishable from an un-run
 * future one) or being lumped in as red "failure". Aligns with CronJobDetail.
 */
export function runLogDotColor(runStatus: string | undefined, jobStatus: string): string {
  switch (runStatus) {
    case "success":
      return "bg-emerald-500"
    case "error":
    case "timeout":
      return "bg-red-500"
    case "running":
      return "bg-blue-500"
    case "empty":
    case "cancelled":
      return "bg-muted-foreground"
    default:
      // No run log for this occurrence (future / not yet run) — color by job status.
      return statusColor(jobStatus)
  }
}

/**
 * Text color + symbol + i18n label key for a run-log status in the calendar
 * day-detail sidebar. C21: aligns with CronJobDetail's per-status branches so
 * `empty` / `cancelled` / `running` are no longer all mislabeled as a red
 * "Error" (`cancelled` reuses `common.cancel`, matching CronJobDetail).
 */
export function runStatusDisplay(runStatus: string): {
  className: string
  symbol: string
  labelKey: string
} {
  switch (runStatus) {
    case "success":
      return { className: "text-emerald-500", symbol: "✓ ", labelKey: "cron.runStatusSuccess" }
    case "running":
      return { className: "text-blue-500", symbol: "", labelKey: "cron.runStatusRunning" }
    case "empty":
      return { className: "text-muted-foreground", symbol: "○ ", labelKey: "cron.runStatusEmpty" }
    case "cancelled":
      return { className: "text-muted-foreground", symbol: "○ ", labelKey: "common.cancel" }
    default:
      // error / timeout / anything else → failure.
      return { className: "text-red-500", symbol: "✕ ", labelKey: "cron.runStatusError" }
  }
}

export function formatSchedule(schedule: CronSchedule, t: (key: string) => string): string {
  switch (schedule.type) {
    case "at":
      return `${t("cron.scheduleAt")}: ${schedule.timestamp ? new Date(schedule.timestamp).toLocaleString() : ""}`
    case "every": {
      const ms = schedule.intervalMs ?? schedule.interval_ms ?? 0
      const secs = ms / 1000
      // §10: sub-minute intervals (legacy rows from before the 1-min floor) show
      // real seconds instead of rounding to "0 minutes".
      if (secs < 60)
        return `${t("cron.scheduleEvery")} ${Math.round(secs)} ${t("cron.unitSeconds")}`
      if (secs < 3600)
        return `${t("cron.scheduleEvery")} ${Math.round(secs / 60)} ${t("cron.unitMinutes")}`
      if (secs < 86400)
        return `${t("cron.scheduleEvery")} ${Math.round(secs / 3600)} ${t("cron.unitHours")}`
      return `${t("cron.scheduleEvery")} ${Math.round(secs / 86400)} ${t("cron.unitDays")}`
    }
    case "cron":
      return `Cron: ${schedule.expression}`
  }
}
