import type { CronSchedule, CronFrequency } from "./CronJobForm.types"

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

export function statusColor(status: string): string {
  switch (status) {
    case "active":
      return "bg-blue-500"
    case "paused":
      return "bg-amber-500"
    case "disabled":
      return "bg-red-500"
    case "completed":
      return "bg-emerald-500"
    case "missed":
      return "bg-orange-500"
    default:
      return "bg-gray-400"
  }
}

export function formatSchedule(schedule: CronSchedule, t: (key: string) => string): string {
  switch (schedule.type) {
    case "at":
      return `${t("cron.scheduleAt")}: ${schedule.timestamp ? new Date(schedule.timestamp).toLocaleString() : ""}`
    case "every": {
      const ms = schedule.intervalMs ?? 0
      const secs = ms / 1000
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
