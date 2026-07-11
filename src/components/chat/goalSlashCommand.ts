import i18n from "@/i18n/i18n"

export const GOAL_SLASH_CONTROL_WORDS = new Set([
  "status",
  "show",
  "help",
  "pause",
  "resume",
  "clear",
  "cancel",
  "evaluate",
  "audit",
  "accept",
  "close",
  "done",
  "strict",
  "needs-strict-evidence",
  "needs_strict_evidence",
])

function readGoalSlashArgs(commandText: string | undefined): string | null {
  const trimmed = commandText?.trim() ?? ""
  const match = /^\/goal(?=\s|$)/i.exec(trimmed)
  if (!match) return null
  return trimmed.slice(match[0].length).trim()
}

export function parseGoalUpsertSlashCommand(commandText: string | undefined): string | null {
  const args = readGoalSlashArgs(commandText)
  if (!args) return null
  if (GOAL_SLASH_CONTROL_WORDS.has(args.toLowerCase())) return null
  return args
}

export function isGoalUpsertSlashCommand(commandText: string | undefined): boolean {
  return parseGoalUpsertSlashCommand(commandText) != null
}

export function parseGoalObjectiveAndCriteria(raw: string): {
  objective: string
  completionCriteria: string
} {
  const markers = ["--criteria", "criteria:", "completion criteria:", "完成标准：", "完成标准:"]
  const lower = raw.toLowerCase()
  for (const marker of markers) {
    const index = lower.indexOf(marker.toLowerCase())
    if (index < 0) continue
    return {
      objective: raw.slice(0, index).trim().replace(/^-+|-+$/g, "").trim(),
      completionCriteria: raw
        .slice(index + marker.length)
        .trim()
        .replace(/^:/, "")
        .trim(),
    }
  }
  return { objective: raw.trim(), completionCriteria: "" }
}

export function goalSlashCommandDisplay(commandText: string): {
  content: string
  mode?: "goal"
} {
  const args = readGoalSlashArgs(commandText)
  if (args == null) return { content: commandText }

  const lowerArgs = args.toLowerCase()
  const goalContent =
    args.length === 0
      ? String(i18n.t("chat.goalSlash.showActive", { defaultValue: "Show active goal" }))
      : lowerArgs === "status" || lowerArgs === "show"
        ? String(i18n.t("chat.goalSlash.showActive", { defaultValue: "Show active goal" }))
        : lowerArgs === "pause"
          ? String(i18n.t("chat.goalSlash.pause", { defaultValue: "Pause active goal" }))
          : lowerArgs === "resume"
            ? String(i18n.t("chat.goalSlash.resume", { defaultValue: "Resume active goal" }))
            : lowerArgs === "clear" || lowerArgs === "cancel"
              ? String(i18n.t("chat.goalSlash.clear", { defaultValue: "Clear active goal" }))
              : lowerArgs === "evaluate" || lowerArgs === "audit"
                ? String(i18n.t("chat.goalSlash.evaluate", { defaultValue: "Evaluate active goal" }))
                : lowerArgs === "accept" || lowerArgs === "close" || lowerArgs === "done"
                  ? String(
                      i18n.t("chat.goalSlash.accept", { defaultValue: "Accept goal completion" }),
                    )
                  : lowerArgs === "strict" ||
                      lowerArgs === "needs-strict-evidence" ||
                      lowerArgs === "needs_strict_evidence"
                    ? String(
                        i18n.t("chat.goalSlash.strict", {
                          defaultValue: "Require stricter evidence",
                        }),
                      )
                    : lowerArgs === "help"
                      ? String(i18n.t("chat.goalSlash.help", { defaultValue: "Goal help" }))
                      : args
  return {
    content: goalContent || String(i18n.t("chat.goalSlash.goal", { defaultValue: "Goal" })),
    mode: "goal",
  }
}
