import { invoke } from "@tauri-apps/api/core"

type LogLevel = "error" | "warn" | "info" | "debug"

interface LogEntry {
  level: LogLevel
  category: string
  source: string
  message: string
  details?: string
  sessionId?: string
}

// Buffer for batching logs to reduce IPC overhead
let logBuffer: LogEntry[] = []
let flushTimer: ReturnType<typeof setTimeout> | null = null
const FLUSH_INTERVAL_MS = 500
const FLUSH_BATCH_SIZE = 20

function scheduleFlush() {
  if (flushTimer) return
  flushTimer = setTimeout(flushLogs, FLUSH_INTERVAL_MS)
}

async function flushLogs() {
  flushTimer = null
  if (logBuffer.length === 0) return

  const entries = logBuffer
  logBuffer = []

  try {
    await invoke("frontend_log_batch", { entries })
  } catch {
    // Fallback: if backend is unavailable, silently drop
    // (we don't want infinite recursion logging about logging failures)
  }
}

function enqueue(entry: LogEntry) {
  logBuffer.push(entry)
  if (logBuffer.length >= FLUSH_BATCH_SIZE) {
    flushLogs()
  } else {
    scheduleFlush()
  }
}

function formatDetails(data: unknown): string | undefined {
  if (data === undefined || data === null) return undefined
  try {
    return typeof data === "string" ? data : JSON.stringify(data)
  } catch {
    return String(data)
  }
}

/**
 * Frontend logger that writes to the backend unified logging system.
 *
 * Usage:
 *   logger.error("chat", "ChatScreen::sendMessage", "Failed to send", { error: e })
 *   logger.info("settings", "ProviderSettings::load", "Loaded 5 providers")
 *   logger.warn("ui", "App::restore", "Session not found", { sessionId })
 *
 * Logs are batched and flushed every 500ms or when batch reaches 20 entries.
 * Also mirrors to console for dev convenience.
 */
export const logger = {
  error(category: string, source: string, message: string, data?: unknown, sessionId?: string) {
    console.error(`[${category}] ${source}:`, message, data ?? "")
    enqueue({ level: "error", category, source, message, details: formatDetails(data), sessionId })
  },

  warn(category: string, source: string, message: string, data?: unknown, sessionId?: string) {
    console.warn(`[${category}] ${source}:`, message, data ?? "")
    enqueue({ level: "warn", category, source, message, details: formatDetails(data), sessionId })
  },

  info(category: string, source: string, message: string, data?: unknown, sessionId?: string) {
    enqueue({ level: "info", category, source, message, details: formatDetails(data), sessionId })
  },

  debug(category: string, source: string, message: string, data?: unknown, sessionId?: string) {
    enqueue({ level: "debug", category, source, message, details: formatDetails(data), sessionId })
  },

  /** Immediately flush any buffered logs (e.g., before page unload) */
  flush: flushLogs,
}
