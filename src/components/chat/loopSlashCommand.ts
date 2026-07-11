import i18n from "@/i18n/i18n"

export const LOOP_SLASH_CONTROL_WORDS = new Set([
  "status",
  "list",
  "show",
  "help",
  "pause",
  "resume",
  "stop",
  "cancel",
])

function readLoopSlashArgs(commandText: string | undefined): string | null {
  const trimmed = commandText?.trim() ?? ""
  const match = /^\/loop(?=\s|$)/i.exec(trimmed)
  if (!match) return null
  return trimmed.slice(match[0].length).trim()
}

export function parseLoopCreateSlashCommand(commandText: string | undefined): string | null {
  const args = readLoopSlashArgs(commandText)
  if (!args) return null
  const first = args.split(/\s+/)[0]?.toLowerCase() ?? ""
  if (LOOP_SLASH_CONTROL_WORDS.has(first)) return null
  return args
}

export function isLoopCreateSlashCommand(commandText: string | undefined): boolean {
  return parseLoopCreateSlashCommand(commandText) != null
}

export function loopSlashCommandDisplay(commandText: string): {
  content: string
  mode?: "loop"
} {
  const args = readLoopSlashArgs(commandText)
  if (args == null) return { content: commandText }

  const first = args.split(/\s+/)[0]?.toLowerCase() ?? ""
  const content =
    args.length === 0
      ? String(i18n.t("chat.loopSlash.startSelfPaced", { defaultValue: "Start self-paced loop" }))
      : first === "status" || first === "list" || first === "show"
        ? String(i18n.t("chat.loopSlash.showLoops", { defaultValue: "Show loops" }))
        : first === "pause"
          ? String(i18n.t("chat.loopSlash.pause", { defaultValue: "Pause loop" }))
          : first === "resume"
            ? String(i18n.t("chat.loopSlash.resume", { defaultValue: "Resume loop" }))
            : first === "stop" || first === "cancel"
              ? String(i18n.t("chat.loopSlash.stop", { defaultValue: "Stop loop" }))
              : first === "help"
                ? String(i18n.t("chat.loopSlash.help", { defaultValue: "Loop help" }))
                : args
  return {
    content: content || String(i18n.t("chat.loopSlash.loop", { defaultValue: "Loop" })),
    mode: "loop",
  }
}
