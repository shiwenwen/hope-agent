import { useState, useEffect, useCallback, useRef } from "react"
import { useTranslation } from "react-i18next"
import { invoke } from "@tauri-apps/api/core"
import { cn } from "@/lib/utils"
import { TooltipProvider, IconTip } from "@/components/ui/tooltip"
import { logger } from "@/lib/logger"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog"
import {
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  ChevronUp,
  Copy,
  Download,
  FileText,
  RefreshCw,
  Search,
  Settings2,
  Trash2,
  X,
} from "lucide-react"
import type { LogConfig, LogEntry, LogFileInfo, LogFilter, LogQueryResult, LogStats } from "./types"

const LEVEL_COLORS: Record<string, string> = {
  error: "bg-red-500/10 text-red-500",
  warn: "bg-yellow-500/10 text-yellow-500",
  info: "bg-blue-500/10 text-blue-500",
  debug: "bg-gray-500/10 text-gray-400",
}

const CATEGORIES = ["agent", "tool", "provider", "system", "session"]
const LEVELS = ["error", "warn", "info", "debug"]

type ViewMode = "structured" | "files"

export default function LogPanel() {
  const { t } = useTranslation()

  // View mode: structured (SQLite) or files (plain text)
  const [viewMode, setViewMode] = useState<ViewMode>("structured")

  // Config
  const [config, setConfig] = useState<LogConfig>({
    enabled: true,
    level: "info",
    maxAgeDays: 30,
    maxSizeMb: 100,
    fileEnabled: true,
    fileMaxSizeMb: 10,
  })
  const [showConfig, setShowConfig] = useState(false)

  // Query state (structured mode)
  const [logs, setLogs] = useState<LogEntry[]>([])
  const [total, setTotal] = useState(0)
  const [page, setPage] = useState(1)
  const [pageSize] = useState(50)
  const [stats, setStats] = useState<LogStats | null>(null)
  const [expandedId, setExpandedId] = useState<number | null>(null)

  // Filter state
  const [filterLevels, setFilterLevels] = useState<string[]>([])
  const [filterCategories, setFilterCategories] = useState<string[]>([])
  const [keyword, setKeyword] = useState("")
  const keywordRef = useRef("")
  const debounceRef = useRef<ReturnType<typeof setTimeout>>()

  // File mode state
  const [logFiles, setLogFiles] = useState<LogFileInfo[]>([])
  const [selectedFile, setSelectedFile] = useState<string | null>(null)
  const [fileContent, setFileContent] = useState("")
  const [fileLoading, setFileLoading] = useState(false)
  const [currentLogPath, setCurrentLogPath] = useState("")

  // Loading
  const [loading, setLoading] = useState(false)

  const buildFilter = useCallback(
    (): LogFilter => ({
      levels: filterLevels.length > 0 ? filterLevels : null,
      categories: filterCategories.length > 0 ? filterCategories : null,
      keyword: keywordRef.current || null,
      sessionId: null,
      startTime: null,
      endTime: null,
    }),
    [filterLevels, filterCategories],
  )

  const fetchLogs = useCallback(async () => {
    setLoading(true)
    try {
      const result = await invoke<LogQueryResult>("query_logs_cmd", {
        filter: buildFilter(),
        page,
        pageSize,
      })
      setLogs(result.logs)
      setTotal(result.total)
    } catch (e) {
      logger.error("settings", "LogPanel::queryLogs", "Failed to query logs", e)
    } finally {
      setLoading(false)
    }
  }, [buildFilter, page, pageSize])

  const fetchStats = useCallback(async () => {
    try {
      const s = await invoke<LogStats>("get_log_stats_cmd")
      setStats(s)
    } catch (e) {
      logger.error("settings", "LogPanel::getStats", "Failed to get log stats", e)
    }
  }, [])

  const fetchConfig = useCallback(async () => {
    try {
      const c = await invoke<LogConfig>("get_log_config_cmd")
      setConfig(c)
    } catch (e) {
      logger.error("settings", "LogPanel::getConfig", "Failed to get log config", e)
    }
  }, [])

  const fetchLogFiles = useCallback(async () => {
    try {
      const files = await invoke<LogFileInfo[]>("list_log_files_cmd")
      setLogFiles(files)
    } catch (e) {
      logger.error("settings", "LogPanel::listFiles", "Failed to list log files", e)
    }
  }, [])

  const fetchCurrentLogPath = useCallback(async () => {
    try {
      const path = await invoke<string>("get_log_file_path_cmd")
      setCurrentLogPath(path)
    } catch (e) {
      logger.error("settings", "LogPanel::getFilePath", "Failed to get log file path", e)
    }
  }, [])

  const fetchFileContent = useCallback(async (filename: string) => {
    setFileLoading(true)
    try {
      const content = await invoke<string>("read_log_file_cmd", {
        filename,
        tailLines: 500,
      })
      setFileContent(content)
    } catch (e) {
      logger.error("settings", "LogPanel::readFile", "Failed to read log file", e)
      setFileContent("")
    } finally {
      setFileLoading(false)
    }
  }, [])

  useEffect(() => {
    fetchConfig()
    fetchStats()
    fetchCurrentLogPath()
  }, [fetchConfig, fetchStats, fetchCurrentLogPath])

  useEffect(() => {
    if (viewMode === "structured") {
      fetchLogs()
    } else {
      fetchLogFiles()
    }
  }, [viewMode, fetchLogs, fetchLogFiles])

  useEffect(() => {
    if (selectedFile) {
      fetchFileContent(selectedFile)
    }
  }, [selectedFile, fetchFileContent])

  const handleKeywordChange = (val: string) => {
    setKeyword(val)
    keywordRef.current = val
    if (debounceRef.current) clearTimeout(debounceRef.current)
    debounceRef.current = setTimeout(() => {
      setPage(1)
      fetchLogs()
    }, 300)
  }

  const toggleLevel = (level: string) => {
    setFilterLevels((prev) =>
      prev.includes(level) ? prev.filter((l) => l !== level) : [...prev, level],
    )
    setPage(1)
  }

  const toggleCategory = (cat: string) => {
    setFilterCategories((prev) =>
      prev.includes(cat) ? prev.filter((c) => c !== cat) : [...prev, cat],
    )
    setPage(1)
  }

  const handleClearLogs = async () => {
    try {
      await invoke("clear_logs_cmd", { beforeDate: null })
      await fetchLogs()
      await fetchStats()
    } catch (e) {
      logger.error("settings", "LogPanel::clearLogs", "Failed to clear logs", e)
    }
  }

  const handleSaveConfig = async (newConfig: LogConfig) => {
    try {
      await invoke("save_log_config_cmd", { config: newConfig })
      setConfig(newConfig)
    } catch (e) {
      logger.error("settings", "LogPanel::saveConfig", "Failed to save log config", e)
    }
  }

  const handleExport = async (format: string) => {
    try {
      const content = await invoke<string>("export_logs_cmd", {
        filter: buildFilter(),
        format,
      })
      const blob = new Blob([content], { type: format === "csv" ? "text/csv" : "application/json" })
      const url = URL.createObjectURL(blob)
      const a = document.createElement("a")
      a.href = url
      a.download = `opencomputer-logs.${format}`
      a.click()
      URL.revokeObjectURL(url)
    } catch (e) {
      logger.error("settings", "LogPanel::export", "Failed to export logs", e)
    }
  }

  const handleCopyPath = async () => {
    if (currentLogPath) {
      await navigator.clipboard.writeText(currentLogPath)
    }
  }

  const totalPages = Math.ceil(total / pageSize)

  const formatTime = (ts: string) => {
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

  const formatSize = (bytes: number) => {
    if (bytes < 1024) return `${bytes} B`
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
  }

  return (
    <div className="flex-1 flex flex-col overflow-hidden">
      {/* Stats + Config Bar */}
      <div className="shrink-0 px-6 pt-4 pb-3 space-y-3">
        <p className="text-xs text-muted-foreground">{t("settings.logsDesc")}</p>

        {/* Log file path hint */}
        {currentLogPath && (
          <div className="flex items-center gap-2">
            <FileText className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
            <code className="text-xs text-muted-foreground font-mono truncate flex-1">
              {currentLogPath}
            </code>
            <TooltipProvider>
              <IconTip label={t("settings.logsCopyPath")}>
                <button
                  onClick={handleCopyPath}
                  className="shrink-0 text-muted-foreground hover:text-foreground transition-colors"
                >
                  <Copy className="h-3.5 w-3.5" />
                </button>
              </IconTip>
            </TooltipProvider>
          </div>
        )}

        {/* Stats summary */}
        {stats && (
          <div className="flex items-center gap-3 flex-wrap">
            <span className="text-xs text-muted-foreground">
              {t("settings.logsTotal")}: {stats.total}
            </span>
            {stats.dbSizeBytes > 0 && (
              <span className="text-xs text-muted-foreground">
                ({formatSize(stats.dbSizeBytes)})
              </span>
            )}
            {LEVELS.map((level) => {
              const count = stats.byLevel[level] || 0
              if (count === 0) return null
              return (
                <span
                  key={level}
                  className={cn(
                    "inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-xs font-medium",
                    LEVEL_COLORS[level],
                  )}
                >
                  {level}: {count}
                </span>
              )
            })}
          </div>
        )}

        {/* View mode tabs + Action buttons */}
        <div className="flex items-center gap-2 flex-wrap">
          {/* View mode toggle */}
          <div className="flex items-center rounded-md border border-border overflow-hidden">
            <button
              onClick={() => setViewMode("structured")}
              className={cn(
                "px-3 py-1 text-xs font-medium transition-colors",
                viewMode === "structured"
                  ? "bg-primary text-primary-foreground"
                  : "bg-secondary/30 text-muted-foreground hover:bg-secondary/50",
              )}
            >
              {t("settings.logsStructured")}
            </button>
            <button
              onClick={() => setViewMode("files")}
              className={cn(
                "px-3 py-1 text-xs font-medium transition-colors",
                viewMode === "files"
                  ? "bg-primary text-primary-foreground"
                  : "bg-secondary/30 text-muted-foreground hover:bg-secondary/50",
              )}
            >
              {t("settings.logsFiles")}
            </button>
          </div>

          <Button
            variant="ghost"
            size="sm"
            onClick={() => setShowConfig(!showConfig)}
            className="gap-1.5 text-xs"
          >
            <Settings2 className="h-3.5 w-3.5" />
            {t("settings.logsConfig")}
            {showConfig ? <ChevronUp className="h-3 w-3" /> : <ChevronDown className="h-3 w-3" />}
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              if (viewMode === "structured") {
                fetchLogs()
                fetchStats()
              } else {
                fetchLogFiles()
                if (selectedFile) fetchFileContent(selectedFile)
              }
            }}
            className="gap-1.5 text-xs"
          >
            <RefreshCw className={cn("h-3.5 w-3.5", loading && "animate-spin")} />
            {t("settings.logsRefresh")}
          </Button>
          <div className="flex-1" />
          {viewMode === "structured" && (
            <>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => handleExport("json")}
                className="gap-1.5 text-xs"
              >
                <Download className="h-3.5 w-3.5" />
                JSON
              </Button>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => handleExport("csv")}
                className="gap-1.5 text-xs"
              >
                <Download className="h-3.5 w-3.5" />
                CSV
              </Button>
            </>
          )}
          <AlertDialog>
            <AlertDialogTrigger asChild>
              <Button
                variant="ghost"
                size="sm"
                className="gap-1.5 text-xs text-red-500 hover:text-red-600"
              >
                <Trash2 className="h-3.5 w-3.5" />
                {t("settings.logsClear")}
              </Button>
            </AlertDialogTrigger>
            <AlertDialogContent>
              <AlertDialogHeader>
                <AlertDialogTitle>{t("settings.logsClearConfirm")}</AlertDialogTitle>
                <AlertDialogDescription>{t("settings.logsClearDesc")}</AlertDialogDescription>
              </AlertDialogHeader>
              <AlertDialogFooter>
                <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
                <AlertDialogAction onClick={handleClearLogs}>
                  {t("common.confirm")}
                </AlertDialogAction>
              </AlertDialogFooter>
            </AlertDialogContent>
          </AlertDialog>
        </div>

        {/* Collapsible config panel */}
        {showConfig && (
          <div className="rounded-lg border border-border p-4 space-y-3 bg-secondary/20">
            <div className="flex items-center justify-between">
              <div>
                <p className="text-sm font-medium">{t("settings.logsEnabled")}</p>
                <p className="text-xs text-muted-foreground">{t("settings.logsEnabledDesc")}</p>
              </div>
              <Switch
                checked={config.enabled}
                onCheckedChange={(checked) => handleSaveConfig({ ...config, enabled: checked })}
              />
            </div>
            <div className="flex items-center justify-between">
              <div>
                <p className="text-sm font-medium">{t("settings.logsFileEnabled")}</p>
                <p className="text-xs text-muted-foreground">{t("settings.logsFileEnabledDesc")}</p>
              </div>
              <Switch
                checked={config.fileEnabled}
                onCheckedChange={(checked) => handleSaveConfig({ ...config, fileEnabled: checked })}
              />
            </div>
            <div className="grid grid-cols-4 gap-3">
              <div>
                <label className="text-xs text-muted-foreground">{t("settings.logsLevel")}</label>
                <select
                  value={config.level}
                  onChange={(e) => handleSaveConfig({ ...config, level: e.target.value })}
                  className="mt-1 w-full rounded-md border border-border bg-background px-2 py-1.5 text-sm"
                >
                  {LEVELS.map((l) => (
                    <option key={l} value={l}>
                      {l}
                    </option>
                  ))}
                </select>
              </div>
              <div>
                <label className="text-xs text-muted-foreground">{t("settings.logsMaxAge")}</label>
                <Input
                  type="number"
                  value={config.maxAgeDays}
                  onChange={(e) =>
                    handleSaveConfig({ ...config, maxAgeDays: parseInt(e.target.value) || 30 })
                  }
                  className="mt-1 h-8 text-sm"
                  min={1}
                  max={365}
                />
              </div>
              <div>
                <label className="text-xs text-muted-foreground">{t("settings.logsMaxSize")}</label>
                <Input
                  type="number"
                  value={config.maxSizeMb}
                  onChange={(e) =>
                    handleSaveConfig({ ...config, maxSizeMb: parseInt(e.target.value) || 100 })
                  }
                  className="mt-1 h-8 text-sm"
                  min={10}
                  max={1000}
                />
              </div>
              <div>
                <label className="text-xs text-muted-foreground">
                  {t("settings.logsFileMaxSize")}
                </label>
                <Input
                  type="number"
                  value={config.fileMaxSizeMb}
                  onChange={(e) =>
                    handleSaveConfig({ ...config, fileMaxSizeMb: parseInt(e.target.value) || 10 })
                  }
                  className="mt-1 h-8 text-sm"
                  min={1}
                  max={100}
                />
              </div>
            </div>
          </div>
        )}

        {/* Filter bar (structured mode only) */}
        {viewMode === "structured" && (
          <div className="flex items-center gap-2 flex-wrap">
            {/* Level filter chips */}
            {LEVELS.map((level) => (
              <button
                key={level}
                onClick={() => toggleLevel(level)}
                className={cn(
                  "px-2 py-0.5 rounded-full text-xs font-medium transition-colors",
                  filterLevels.includes(level)
                    ? LEVEL_COLORS[level]
                    : "bg-secondary/40 text-muted-foreground hover:bg-secondary/60",
                )}
              >
                {level}
              </button>
            ))}
            <span className="w-px h-4 bg-border" />
            {/* Category filter chips */}
            {CATEGORIES.map((cat) => (
              <button
                key={cat}
                onClick={() => toggleCategory(cat)}
                className={cn(
                  "px-2 py-0.5 rounded-full text-xs font-medium transition-colors",
                  filterCategories.includes(cat)
                    ? "bg-primary/10 text-primary"
                    : "bg-secondary/40 text-muted-foreground hover:bg-secondary/60",
                )}
              >
                {cat}
              </button>
            ))}
            <span className="w-px h-4 bg-border" />
            {/* Keyword search */}
            <div className="relative flex-1 min-w-[160px] max-w-[300px]">
              <Search className="absolute left-2 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground" />
              <Input
                value={keyword}
                onChange={(e) => handleKeywordChange(e.target.value)}
                placeholder={t("settings.logsSearch")}
                className="h-7 pl-7 pr-7 text-xs"
              />
              {keyword && (
                <button
                  onClick={() => handleKeywordChange("")}
                  className="absolute right-2 top-1/2 -translate-y-1/2"
                >
                  <X className="h-3 w-3 text-muted-foreground hover:text-foreground" />
                </button>
              )}
            </div>
            {(filterLevels.length > 0 || filterCategories.length > 0 || keyword) && (
              <button
                onClick={() => {
                  setFilterLevels([])
                  setFilterCategories([])
                  handleKeywordChange("")
                }}
                className="text-xs text-muted-foreground hover:text-foreground"
              >
                {t("settings.logsClearFilter")}
              </button>
            )}
          </div>
        )}
      </div>

      {/* Content area */}
      {viewMode === "structured" ? (
        <>
          {/* Structured log list */}
          <div className="flex-1 overflow-y-auto px-6">
            {logs.length === 0 ? (
              <div className="flex items-center justify-center h-32 text-sm text-muted-foreground">
                {loading ? t("settings.logsLoading") : t("settings.logsEmpty")}
              </div>
            ) : (
              <div className="space-y-0.5">
                {logs.map((log) => (
                  <div key={log.id}>
                    <button
                      onClick={() => setExpandedId(expandedId === log.id ? null : log.id)}
                      className="w-full flex items-center gap-2 px-2 py-1.5 rounded text-left text-xs hover:bg-secondary/40 transition-colors"
                    >
                      <span className="shrink-0 w-[110px] text-muted-foreground font-mono">
                        {formatTime(log.timestamp)}
                      </span>
                      <span
                        className={cn(
                          "shrink-0 w-[46px] text-center rounded px-1 py-0.5 font-medium",
                          LEVEL_COLORS[log.level] || "bg-secondary text-foreground",
                        )}
                      >
                        {log.level}
                      </span>
                      <span className="shrink-0 w-[64px] text-muted-foreground truncate">
                        {log.category}
                      </span>
                      <span className="shrink-0 w-[140px] text-muted-foreground truncate font-mono">
                        {log.source}
                      </span>
                      <span className="flex-1 truncate text-foreground">{log.message}</span>
                      {log.details && (
                        <ChevronDown
                          className={cn(
                            "h-3 w-3 shrink-0 text-muted-foreground transition-transform",
                            expandedId === log.id && "rotate-180",
                          )}
                        />
                      )}
                    </button>
                    {expandedId === log.id && log.details && (
                      <div className="ml-[112px] mb-1 px-3 py-2 rounded bg-secondary/30 text-xs font-mono overflow-x-auto">
                        <pre className="whitespace-pre-wrap break-all text-muted-foreground">
                          {(() => {
                            try {
                              return JSON.stringify(JSON.parse(log.details), null, 2)
                            } catch {
                              return log.details
                            }
                          })()}
                        </pre>
                        {log.sessionId && (
                          <p className="mt-1 text-muted-foreground/70">session: {log.sessionId}</p>
                        )}
                      </div>
                    )}
                  </div>
                ))}
              </div>
            )}
          </div>

          {/* Pagination */}
          {totalPages > 1 && (
            <div className="shrink-0 px-6 py-2 border-t border-border flex items-center justify-between">
              <span className="text-xs text-muted-foreground">
                {t("settings.logsPagination", { page, totalPages, total })}
              </span>
              <div className="flex items-center gap-1">
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={page <= 1}
                  onClick={() => setPage(page - 1)}
                  className="h-7 w-7 p-0"
                >
                  <ChevronLeft className="h-4 w-4" />
                </Button>
                <span className="text-xs text-muted-foreground px-2">
                  {page} / {totalPages}
                </span>
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={page >= totalPages}
                  onClick={() => setPage(page + 1)}
                  className="h-7 w-7 p-0"
                >
                  <ChevronRight className="h-4 w-4" />
                </Button>
              </div>
            </div>
          )}
        </>
      ) : (
        /* File mode */
        <div className="flex-1 flex overflow-hidden">
          {/* File list sidebar */}
          <div className="w-[220px] shrink-0 border-r border-border overflow-y-auto">
            {logFiles.length === 0 ? (
              <div className="flex items-center justify-center h-32 text-xs text-muted-foreground">
                {t("settings.logsNoFiles")}
              </div>
            ) : (
              <div className="py-1">
                {logFiles.map((file) => (
                  <button
                    key={file.name}
                    onClick={() => setSelectedFile(file.name)}
                    className={cn(
                      "w-full px-3 py-2 text-left text-xs hover:bg-secondary/40 transition-colors",
                      selectedFile === file.name && "bg-secondary/60",
                    )}
                  >
                    <p className="font-medium truncate">{file.name}</p>
                    <p className="text-muted-foreground">{formatSize(file.sizeBytes)}</p>
                  </button>
                ))}
              </div>
            )}
          </div>

          {/* File content viewer */}
          <div className="flex-1 overflow-y-auto">
            {selectedFile ? (
              fileLoading ? (
                <div className="flex items-center justify-center h-32 text-sm text-muted-foreground">
                  {t("settings.logsLoading")}
                </div>
              ) : (
                <pre className="px-4 py-3 text-xs font-mono text-muted-foreground whitespace-pre-wrap break-all leading-relaxed">
                  {fileContent || t("settings.logsEmpty")}
                </pre>
              )
            ) : (
              <div className="flex items-center justify-center h-32 text-sm text-muted-foreground">
                {t("settings.logsSelectFile")}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  )
}
