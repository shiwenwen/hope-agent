import { useEffect, useRef, useState, useEffectEvent } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type {
  LogConfig,
  LogEntry,
  LogFileInfo,
  LogFilter,
  LogQueryResult,
  LogStats,
} from "../types"
import LogActionBar from "./LogActionBar"
import LogToolbar from "./LogToolbar"
import LogTable from "./LogTable"
import LogConfigSection from "./LogConfigSection"

type ViewMode = "structured" | "files"

export default function LogPanel() {
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

  // Filter state
  const [filterLevels, setFilterLevels] = useState<string[]>([])
  const [filterCategories, setFilterCategories] = useState<string[]>([])
  const [keyword, setKeyword] = useState("")
  const keywordRef = useRef("")
  const debounceRef = useRef<ReturnType<typeof setTimeout>>(undefined)

  // File mode state
  const [logFiles, setLogFiles] = useState<LogFileInfo[]>([])
  const [selectedFile, setSelectedFile] = useState<string | null>(null)
  const [fileContent, setFileContent] = useState("")
  const [fileLoading, setFileLoading] = useState(false)
  const [currentLogPath, setCurrentLogPath] = useState("")

  // Loading
  const [loading, setLoading] = useState(false)

  const buildFilter = (): LogFilter => ({
    levels: filterLevels.length > 0 ? filterLevels : null,
    categories: filterCategories.length > 0 ? filterCategories : null,
    keyword: keywordRef.current || null,
    sessionId: null,
    startTime: null,
    endTime: null,
  })

  const fetchLogs = async () => {
    setLoading(true)
    try {
      const result = await getTransport().call<LogQueryResult>("query_logs_cmd", {
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
  }
  const fetchLogsEffectEvent = useEffectEvent(fetchLogs)

  const fetchStats = async () => {
    try {
      const s = await getTransport().call<LogStats>("get_log_stats_cmd")
      setStats(s)
    } catch (e) {
      logger.error("settings", "LogPanel::getStats", "Failed to get log stats", e)
    }
  }
  const fetchStatsEffectEvent = useEffectEvent(fetchStats)

  const fetchConfig = async () => {
    try {
      const c = await getTransport().call<LogConfig>("get_log_config_cmd")
      setConfig(c)
    } catch (e) {
      logger.error("settings", "LogPanel::getConfig", "Failed to get log config", e)
    }
  }
  const fetchConfigEffectEvent = useEffectEvent(fetchConfig)

  const fetchLogFiles = async () => {
    try {
      const files = await getTransport().call<LogFileInfo[]>("list_log_files_cmd")
      setLogFiles(files)
    } catch (e) {
      logger.error("settings", "LogPanel::listFiles", "Failed to list log files", e)
    }
  }
  const fetchLogFilesEffectEvent = useEffectEvent(fetchLogFiles)

  const fetchCurrentLogPath = async () => {
    try {
      const path = await getTransport().call<string>("get_log_file_path_cmd")
      setCurrentLogPath(path)
    } catch (e) {
      logger.error("settings", "LogPanel::getFilePath", "Failed to get log file path", e)
    }
  }
  const fetchCurrentLogPathEffectEvent = useEffectEvent(fetchCurrentLogPath)

  const fetchFileContent = async (filename: string) => {
    setFileLoading(true)
    try {
      const content = await getTransport().call<string>("read_log_file_cmd", {
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
  }
  const fetchFileContentEffectEvent = useEffectEvent(fetchFileContent)

  useEffect(() => {
    fetchConfigEffectEvent()
    fetchStatsEffectEvent()
    fetchCurrentLogPathEffectEvent()
  }, [])

  useEffect(() => {
    if (viewMode === "structured") {
      fetchLogsEffectEvent()
    } else {
      fetchLogFilesEffectEvent()
    }
  }, [viewMode, filterLevels, filterCategories, page, pageSize])

  useEffect(() => {
    if (selectedFile) {
      fetchFileContentEffectEvent(selectedFile)
    }
  }, [selectedFile])

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

  const handleClearAllFilters = () => {
    setFilterLevels([])
    setFilterCategories([])
    handleKeywordChange("")
  }

  const handleClearLogs = async () => {
    try {
      await getTransport().call("clear_logs_cmd", { beforeDate: null })
      await fetchLogs()
      await fetchStats()
    } catch (e) {
      logger.error("settings", "LogPanel::clearLogs", "Failed to clear logs", e)
    }
  }

  const handleSaveConfig = async (newConfig: LogConfig) => {
    try {
      await getTransport().call("save_log_config_cmd", { config: newConfig })
      setConfig(newConfig)
    } catch (e) {
      logger.error("settings", "LogPanel::saveConfig", "Failed to save log config", e)
    }
  }

  const handleExport = async (format: string) => {
    try {
      const content = await getTransport().call<string>("export_logs_cmd", {
        filter: buildFilter(),
        format,
      })
      const blob = new Blob([content], { type: format === "csv" ? "text/csv" : "application/json" })
      const url = URL.createObjectURL(blob)
      const a = document.createElement("a")
      a.href = url
      a.download = `hope-agent-logs.${format}`
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

  const handleRefresh = () => {
    if (viewMode === "structured") {
      fetchLogs()
      fetchStats()
    } else {
      fetchLogFiles()
      if (selectedFile) fetchFileContent(selectedFile)
    }
  }

  return (
    <div className="flex-1 flex flex-col overflow-hidden">
      {/* Stats + Config Bar */}
      <div className="shrink-0 px-6 pt-4 pb-3 space-y-3">
        <LogActionBar
          viewMode={viewMode}
          showConfig={showConfig}
          loading={loading}
          stats={stats}
          currentLogPath={currentLogPath}
          onViewModeChange={setViewMode}
          onToggleConfig={() => setShowConfig(!showConfig)}
          onRefresh={handleRefresh}
          onExport={handleExport}
          onClearLogs={handleClearLogs}
          onCopyPath={handleCopyPath}
        />

        {/* Collapsible config panel */}
        {showConfig && <LogConfigSection config={config} onSaveConfig={handleSaveConfig} />}

        {/* Filter bar (structured mode only) */}
        {viewMode === "structured" && (
          <LogToolbar
            filterLevels={filterLevels}
            filterCategories={filterCategories}
            keyword={keyword}
            onToggleLevel={toggleLevel}
            onToggleCategory={toggleCategory}
            onKeywordChange={handleKeywordChange}
            onClearAll={handleClearAllFilters}
          />
        )}
      </div>

      {/* Content area */}
      <LogTable
        viewMode={viewMode}
        logs={logs}
        total={total}
        page={page}
        pageSize={pageSize}
        loading={loading}
        logFiles={logFiles}
        selectedFile={selectedFile}
        fileContent={fileContent}
        fileLoading={fileLoading}
        onPageChange={setPage}
        onSelectFile={setSelectedFile}
      />
    </div>
  )
}
