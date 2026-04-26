import { useState } from "react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { formatBytes } from "@/lib/format"
import { Button } from "@/components/ui/button"
import { ChevronDown, ChevronLeft, ChevronRight } from "lucide-react"
import { LEVEL_COLORS } from "./constants"
import { formatTime } from "./constants"
import type { LogEntry, LogFileInfo } from "../types"

type ViewMode = "structured" | "files"

interface LogTableProps {
  viewMode: ViewMode
  logs: LogEntry[]
  total: number
  page: number
  pageSize: number
  loading: boolean
  logFiles: LogFileInfo[]
  selectedFile: string | null
  fileContent: string
  fileLoading: boolean
  onPageChange: (page: number) => void
  onSelectFile: (name: string) => void
}

export default function LogTable({
  viewMode,
  logs,
  total,
  page,
  pageSize,
  loading,
  logFiles,
  selectedFile,
  fileContent,
  fileLoading,
  onPageChange,
  onSelectFile,
}: LogTableProps) {
  const { t } = useTranslation()
  const [expandedId, setExpandedId] = useState<number | null>(null)
  const totalPages = Math.ceil(total / pageSize)

  if (viewMode === "structured") {
    return (
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
                onClick={() => onPageChange(page - 1)}
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
                onClick={() => onPageChange(page + 1)}
                className="h-7 w-7 p-0"
              >
                <ChevronRight className="h-4 w-4" />
              </Button>
            </div>
          </div>
        )}
      </>
    )
  }

  // File mode
  return (
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
                onClick={() => onSelectFile(file.name)}
                className={cn(
                  "w-full px-3 py-2 text-left text-xs hover:bg-secondary/40 transition-colors",
                  selectedFile === file.name && "bg-secondary/60",
                )}
              >
                <p className="font-medium truncate">{file.name}</p>
                <p className="text-muted-foreground">{formatBytes(file.sizeBytes)}</p>
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
  )
}
