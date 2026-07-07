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
  const first = args.split(/\s+/)[0]?.toLowerCase()
  if (GOAL_SLASH_CONTROL_WORDS.has(first)) return null
  return args
}

export function isGoalUpsertSlashCommand(commandText: string | undefined): boolean {
  return parseGoalUpsertSlashCommand(commandText) != null
}

export function goalSlashCommandDisplay(commandText: string): {
  content: string
  mode?: "goal"
} {
  const args = readGoalSlashArgs(commandText)
  if (args == null) return { content: commandText }

  const [first = ""] = args.split(/\s+/)
  const goalContent =
    args.length === 0
      ? "Show active goal"
      : first === "status" || first === "show"
        ? "Show active goal"
        : first === "pause"
          ? "Pause active goal"
          : first === "resume"
            ? "Resume active goal"
            : first === "clear" || first === "cancel"
              ? "Clear active goal"
              : first === "evaluate" || first === "audit"
                ? "Evaluate active goal"
                : first === "accept" || first === "close" || first === "done"
                  ? "Accept goal completion"
                  : first === "strict" ||
                      first === "needs-strict-evidence" ||
                      first === "needs_strict_evidence"
                    ? "Require stricter evidence"
                    : first === "help"
                      ? "Goal help"
                      : args
  return { content: goalContent || "Goal", mode: "goal" }
}
