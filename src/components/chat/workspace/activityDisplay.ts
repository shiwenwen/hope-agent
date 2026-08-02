import type { TFunction } from "i18next"

import type { AutonomyActivity } from "./useGoal"

const JOB_ACTIVITY_HEADLINES = new Set(["waiting_job_approval", "waiting_background_work"])

/**
 * Convert background-job wire identifiers into user-facing labels. Other
 * activity sources contain user-authored text and must remain unchanged.
 */
export function autonomyActivitySourceLabel(
  t: TFunction,
  activity: AutonomyActivity | null | undefined,
  label: string | null | undefined,
): string {
  const value = label?.trim() ?? ""
  if (!value || !activity || !JOB_ACTIVITY_HEADLINES.has(activity.headlineCode)) return value

  if (value === "subagent:batch") {
    return String(t("backgroundJobs.kindGroup", { defaultValue: "任务组" }))
  }
  if (value === "subagent" || value.startsWith("subagent:")) {
    return String(t("backgroundJobs.kindSubagent", { defaultValue: "子 Agent" }))
  }

  return String(t(`tools.${value}`, { defaultValue: value }))
}
