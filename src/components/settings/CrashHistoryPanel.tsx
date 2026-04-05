import { useState, useEffect, useCallback } from "react"
import { useTranslation } from "react-i18next"
import { getTransport } from "@/lib/transport-provider"
import { Button } from "@/components/ui/button"
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
import { IconTip } from "@/components/ui/tooltip"
import {
  Archive,
  ChevronDown,
  ChevronUp,
  Download,
  RefreshCw,
  RotateCcw,
  Trash2,
} from "lucide-react"
import { cn } from "@/lib/utils"
import { logger } from "@/lib/logger"

interface CrashEntry {
  timestamp: string
  exit_code: number
  signal: string | null
  crash_count_session: number
  diagnosis_run: boolean
  diagnosis_result: DiagnosisResult | null
}

interface DiagnosisResult {
  cause: string
  severity: string
  user_actionable: boolean
  recommendations: string[]
  auto_fix_applied: string[]
  provider_used: string | null
}

interface CrashJournal {
  crashes: CrashEntry[]
  total_crashes: number
  last_backup: string | null
}

interface BackupInfo {
  name: string
  path: string
  createdAt: number
}

const SEVERITY_COLORS: Record<string, string> = {
  critical: "bg-red-500/10 text-red-500 border-red-500/20",
  high: "bg-orange-500/10 text-orange-500 border-orange-500/20",
  medium: "bg-yellow-500/10 text-yellow-500 border-yellow-500/20",
  low: "bg-blue-500/10 text-blue-500 border-blue-500/20",
  unknown: "bg-gray-500/10 text-gray-400 border-gray-500/20",
}

export default function CrashHistoryPanel() {
  const { t } = useTranslation()
  const [journal, setJournal] = useState<CrashJournal | null>(null)
  const [backups, setBackups] = useState<BackupInfo[]>([])
  const [expandedIdx, setExpandedIdx] = useState<number | null>(null)
  const [loading, setLoading] = useState(true)
  const [backupLoading, setBackupLoading] = useState(false)
  const [guardianEnabled, setGuardianEnabled] = useState(true)

  const loadData = useCallback(async () => {
    setLoading(true)
    try {
      const [journalData, backupData, enabled] = await Promise.all([
        getTransport().call<CrashJournal>("get_crash_history"),
        getTransport().call<BackupInfo[]>("list_backups_cmd"),
        getTransport().call<boolean>("get_guardian_enabled"),
      ])
      setJournal(journalData as CrashJournal)
      setBackups(backupData as BackupInfo[])
      setGuardianEnabled(enabled)
    } catch (e) {
      logger.error("health", "CrashHistoryPanel", `Failed to load crash history: ${e}`)
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    loadData()
  }, [loadData])

  const handleClearHistory = async () => {
    try {
      await getTransport().call("clear_crash_history")
      setJournal(null)
      loadData()
    } catch (e) {
      logger.error("health", "CrashHistoryPanel", `Failed to clear crash history: ${e}`)
    }
  }

  const handleCreateBackup = async () => {
    setBackupLoading(true)
    try {
      await getTransport().call<string>("create_backup_cmd")
      loadData()
    } catch (e) {
      logger.error("health", "CrashHistoryPanel", `Failed to create backup: ${e}`)
    } finally {
      setBackupLoading(false)
    }
  }

  const handleRestoreBackup = async (name: string) => {
    try {
      await getTransport().call("restore_backup_cmd", { name })
      loadData()
    } catch (e) {
      logger.error("health", "CrashHistoryPanel", `Failed to restore backup: ${e}`)
    }
  }

  const handleRestart = async () => {
    try {
      await getTransport().call("request_app_restart")
    } catch (e) {
      logger.error("health", "CrashHistoryPanel", `Failed to request restart: ${e}`)
    }
  }

  const handleGuardianToggle = async (enabled: boolean) => {
    setGuardianEnabled(enabled)
    try {
      await getTransport().call("set_guardian_enabled", { enabled })
    } catch (e) {
      setGuardianEnabled(!enabled)
      logger.error("health", "CrashHistoryPanel", `Failed to set guardian enabled: ${e}`)
    }
  }

  const formatTime = (timestamp: string) => {
    try {
      return new Date(timestamp).toLocaleString()
    } catch {
      return timestamp
    }
  }

  const crashes = journal?.crashes?.slice().reverse() ?? []

  return (
    <div className="flex-1 overflow-y-auto p-6 space-y-6">
      {/* Guardian Toggle */}
      <div className="flex items-center justify-between rounded-lg border bg-card p-4">
        <div className="space-y-0.5">
          <label htmlFor="guardian-toggle" className="text-sm font-medium">
            {t("health.guardianEnabled")}
          </label>
          <p className="text-xs text-muted-foreground">{t("health.guardianEnabledDesc")}</p>
        </div>
        <Switch
          id="guardian-toggle"
          checked={guardianEnabled}
          onCheckedChange={handleGuardianToggle}
        />
      </div>

      {/* Header Actions */}
      <div className="flex items-center gap-2 flex-wrap">
        <IconTip label={t("health.refreshTooltip")}>
            <Button variant="outline" size="sm" onClick={loadData} disabled={loading}>
              <RefreshCw className={cn("h-4 w-4 mr-1.5", loading && "animate-spin")} />
              {t("health.refresh")}
            </Button>
          </IconTip>

          <IconTip label={t("health.createBackupTooltip")}>
            <Button
              variant="outline"
              size="sm"
              onClick={handleCreateBackup}
              disabled={backupLoading}
            >
              <Download className="h-4 w-4 mr-1.5" />
              {t("health.createBackup")}
            </Button>
          </IconTip>

          <IconTip label={t("health.restartTooltip")}>
            <Button variant="outline" size="sm" onClick={handleRestart}>
              <RotateCcw className="h-4 w-4 mr-1.5" />
              {t("health.restart")}
            </Button>
          </IconTip>

        {crashes.length > 0 && (
          <AlertDialog>
            <AlertDialogTrigger asChild>
              <Button variant="outline" size="sm" className="text-destructive">
                <Trash2 className="h-4 w-4 mr-1.5" />
                {t("health.clearHistory")}
              </Button>
            </AlertDialogTrigger>
            <AlertDialogContent>
              <AlertDialogHeader>
                <AlertDialogTitle>{t("health.clearConfirmTitle")}</AlertDialogTitle>
                <AlertDialogDescription>{t("health.clearConfirmDesc")}</AlertDialogDescription>
              </AlertDialogHeader>
              <AlertDialogFooter>
                <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
                <AlertDialogAction onClick={handleClearHistory}>
                  {t("common.confirm")}
                </AlertDialogAction>
              </AlertDialogFooter>
            </AlertDialogContent>
          </AlertDialog>
        )}
      </div>

      {/* Stats Summary */}
      {journal && journal.total_crashes > 0 && (
        <div className="grid grid-cols-3 gap-4">
          <div className="rounded-lg border bg-card p-4">
            <div className="text-2xl font-bold">{journal.total_crashes}</div>
            <div className="text-sm text-muted-foreground">{t("health.totalCrashes")}</div>
          </div>
          <div className="rounded-lg border bg-card p-4">
            <div className="text-2xl font-bold">{journal.crashes.length}</div>
            <div className="text-sm text-muted-foreground">{t("health.recentCrashes")}</div>
          </div>
          <div className="rounded-lg border bg-card p-4">
            <div className="text-sm font-medium truncate">
              {journal.last_backup ? formatTime(journal.last_backup) : "-"}
            </div>
            <div className="text-sm text-muted-foreground">{t("health.lastBackup")}</div>
          </div>
        </div>
      )}

      {/* Crash History */}
      <div className="space-y-2">
        <h3 className="text-sm font-medium text-foreground">{t("health.crashHistory")}</h3>
        {crashes.length === 0 ? (
          <div className="text-sm text-muted-foreground py-8 text-center">
            {t("health.noCrashes")}
          </div>
        ) : (
          <div className="space-y-2">
            {crashes.map((entry, idx) => {
              const isExpanded = expandedIdx === idx
              return (
                <div key={idx} className="rounded-lg border bg-card">
                  <button
                    className="w-full flex items-center gap-3 px-4 py-3 text-left"
                    onClick={() => setExpandedIdx(isExpanded ? null : idx)}
                  >
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="text-sm font-medium">{formatTime(entry.timestamp)}</span>
                        <span className="text-xs text-muted-foreground">
                          exit={entry.exit_code}
                          {entry.signal && ` (${entry.signal})`}
                        </span>
                        {entry.diagnosis_result && (
                          <span
                            className={cn(
                              "text-xs px-1.5 py-0.5 rounded border",
                              SEVERITY_COLORS[entry.diagnosis_result.severity] ??
                                SEVERITY_COLORS.unknown,
                            )}
                          >
                            {entry.diagnosis_result.severity}
                          </span>
                        )}
                      </div>
                      {entry.diagnosis_result && (
                        <div className="text-xs text-muted-foreground mt-0.5 truncate">
                          {entry.diagnosis_result.cause}
                        </div>
                      )}
                    </div>
                    {isExpanded ? (
                      <ChevronUp className="h-4 w-4 shrink-0 text-muted-foreground" />
                    ) : (
                      <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground" />
                    )}
                  </button>

                  {isExpanded && entry.diagnosis_result && (
                    <div className="px-4 pb-4 space-y-3 border-t">
                      <div className="pt-3">
                        <div className="text-xs font-medium text-muted-foreground mb-1">
                          {t("health.cause")}
                        </div>
                        <div className="text-sm">{entry.diagnosis_result.cause}</div>
                      </div>
                      {entry.diagnosis_result.recommendations.length > 0 && (
                        <div>
                          <div className="text-xs font-medium text-muted-foreground mb-1">
                            {t("health.recommendations")}
                          </div>
                          <ul className="list-disc list-inside space-y-0.5">
                            {entry.diagnosis_result.recommendations.map((rec, i) => (
                              <li key={i} className="text-sm">
                                {rec}
                              </li>
                            ))}
                          </ul>
                        </div>
                      )}
                      {entry.diagnosis_result.auto_fix_applied.length > 0 && (
                        <div>
                          <div className="text-xs font-medium text-muted-foreground mb-1">
                            {t("health.autoFixes")}
                          </div>
                          <ul className="list-disc list-inside space-y-0.5">
                            {entry.diagnosis_result.auto_fix_applied.map((fix, i) => (
                              <li key={i} className="text-sm text-green-500">
                                {fix}
                              </li>
                            ))}
                          </ul>
                        </div>
                      )}
                      {entry.diagnosis_result.provider_used && (
                        <div className="text-xs text-muted-foreground">
                          {t("health.diagnosedBy")}: {entry.diagnosis_result.provider_used}
                        </div>
                      )}
                    </div>
                  )}
                </div>
              )
            })}
          </div>
        )}
      </div>

      {/* Backups */}
      <div className="space-y-2">
        <h3 className="text-sm font-medium text-foreground">{t("health.backups")}</h3>
        {backups.length === 0 ? (
          <div className="text-sm text-muted-foreground py-4 text-center">
            {t("health.noBackups")}
          </div>
        ) : (
          <div className="space-y-2">
            {backups.map((backup) => (
              <div
                key={backup.name}
                className="flex items-center gap-3 rounded-lg border bg-card px-4 py-3"
              >
                <Archive className="h-4 w-4 shrink-0 text-muted-foreground" />
                <div className="flex-1 min-w-0">
                  <div className="text-sm font-medium truncate">{backup.name}</div>
                  <div className="text-xs text-muted-foreground">
                    {new Date(backup.createdAt * 1000).toLocaleString()}
                  </div>
                </div>
                <AlertDialog>
                  <AlertDialogTrigger asChild>
                    <Button variant="outline" size="sm">
                      <RotateCcw className="h-3.5 w-3.5 mr-1" />
                      {t("health.restore")}
                    </Button>
                  </AlertDialogTrigger>
                  <AlertDialogContent>
                    <AlertDialogHeader>
                      <AlertDialogTitle>{t("health.restoreConfirmTitle")}</AlertDialogTitle>
                      <AlertDialogDescription>
                        {t("health.restoreConfirmDesc")}
                      </AlertDialogDescription>
                    </AlertDialogHeader>
                    <AlertDialogFooter>
                      <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
                      <AlertDialogAction onClick={() => handleRestoreBackup(backup.name)}>
                        {t("health.restore")}
                      </AlertDialogAction>
                    </AlertDialogFooter>
                  </AlertDialogContent>
                </AlertDialog>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}
