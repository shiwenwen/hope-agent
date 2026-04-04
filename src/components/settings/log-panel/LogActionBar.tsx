import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { IconTip } from "@/components/ui/tooltip"
import { Button } from "@/components/ui/button"
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
  ChevronUp,
  Copy,
  Download,
  FileText,
  RefreshCw,
  Settings2,
  Trash2,
} from "lucide-react"
import { LEVELS, LEVEL_COLORS, formatSize } from "./constants"
import type { LogStats } from "../types"

type ViewMode = "structured" | "files"

interface LogActionBarProps {
  viewMode: ViewMode
  showConfig: boolean
  loading: boolean
  stats: LogStats | null
  currentLogPath: string
  onViewModeChange: (mode: ViewMode) => void
  onToggleConfig: () => void
  onRefresh: () => void
  onExport: (format: string) => void
  onClearLogs: () => void
  onCopyPath: () => void
}

export default function LogActionBar({
  viewMode,
  showConfig,
  loading,
  stats,
  currentLogPath,
  onViewModeChange,
  onToggleConfig,
  onRefresh,
  onExport,
  onClearLogs,
  onCopyPath,
}: LogActionBarProps) {
  const { t } = useTranslation()

  return (
    <>
      <p className="text-xs text-muted-foreground">{t("settings.logsDesc")}</p>

      {/* Log file path hint */}
      {currentLogPath && (
        <div className="flex items-center gap-2">
          <FileText className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
          <code className="text-xs text-muted-foreground font-mono truncate flex-1">
            {currentLogPath}
          </code>
          <IconTip label={t("settings.logsCopyPath")}>
            <button
              onClick={onCopyPath}
              className="shrink-0 text-muted-foreground hover:text-foreground transition-colors"
            >
              <Copy className="h-3.5 w-3.5" />
            </button>
          </IconTip>
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
            onClick={() => onViewModeChange("structured")}
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
            onClick={() => onViewModeChange("files")}
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
          onClick={onToggleConfig}
          className="gap-1.5 text-xs"
        >
          <Settings2 className="h-3.5 w-3.5" />
          {t("settings.logsConfig")}
          {showConfig ? <ChevronUp className="h-3 w-3" /> : <ChevronDown className="h-3 w-3" />}
        </Button>
        <Button
          variant="ghost"
          size="sm"
          onClick={onRefresh}
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
              onClick={() => onExport("json")}
              className="gap-1.5 text-xs"
            >
              <Download className="h-3.5 w-3.5" />
              JSON
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => onExport("csv")}
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
              <AlertDialogAction onClick={onClearLogs}>
                {t("common.confirm")}
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      </div>
    </>
  )
}
