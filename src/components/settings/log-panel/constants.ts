export const LEVEL_COLORS: Record<string, string> = {
  error: "bg-red-500/10 text-red-500",
  warn: "bg-yellow-500/10 text-yellow-500",
  info: "bg-blue-500/10 text-blue-500",
  debug: "bg-gray-500/10 text-gray-400",
}

export const CATEGORIES = ["agent", "tool", "provider", "system", "session"]
export const LEVELS = ["error", "warn", "info", "debug"]

export const formatTime = (ts: string) => {
  try {
    const d = new Date(ts)
    return d.toLocaleString(undefined, {
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    })
  } catch {
    return ts
  }
}
