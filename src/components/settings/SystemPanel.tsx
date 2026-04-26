import { useState, useEffect, useCallback } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { Switch } from "@/components/ui/switch"
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
} from "@/components/ui/alert-dialog"
import { IconTip } from "@/components/ui/tooltip"
import { History, RefreshCw, RotateCcw, Loader2 } from "lucide-react"
import { cn } from "@/lib/utils"

interface AutosaveEntry {
  id: string
  timestamp: string
  kind: string
  category: string
  source: string
}

export default function SystemPanel() {
  const { t } = useTranslation()
  const [autostart, setAutostart] = useState(false)
  const [loaded, setLoaded] = useState(false)

  // ── Config Backups ──
  const [backups, setBackups] = useState<AutosaveEntry[]>([])
  const [backupsLoading, setBackupsLoading] = useState(false)
  const [pendingRestore, setPendingRestore] = useState<AutosaveEntry | null>(null)
  const [restoringId, setRestoringId] = useState<string | null>(null)
  const [restoreError, setRestoreError] = useState<string | null>(null)

  useEffect(() => {
    getTransport().call<boolean>("get_autostart_enabled")
      .then((enabled) => {
        setAutostart(enabled)
        setLoaded(true)
      })
      .catch((e) => {
        logger.error("settings", "SystemPanel::load", "Failed to get autostart status", e)
        setLoaded(true)
      })
  }, [])

  const loadBackups = useCallback(async () => {
    setBackupsLoading(true)
    try {
      const list = await getTransport().call<AutosaveEntry[]>("list_settings_backups_cmd")
      setBackups(list)
    } catch (e) {
      logger.error("settings", "SystemPanel::loadBackups", "Failed to list settings backups", e)
    } finally {
      setBackupsLoading(false)
    }
  }, [])

  useEffect(() => {
    loadBackups()
  }, [loadBackups])

  async function toggleAutostart() {
    const next = !autostart
    setAutostart(next)
    try {
      await getTransport().call("set_autostart_enabled", { enabled: next })
    } catch (e) {
      setAutostart(!next)
      logger.error("settings", "SystemPanel::toggle", "Failed to set autostart", e)
    }
  }

  async function handleRestore() {
    if (!pendingRestore) return
    setRestoringId(pendingRestore.id)
    setRestoreError(null)
    try {
      await getTransport().call("restore_settings_backup_cmd", { id: pendingRestore.id })
      await loadBackups()
    } catch (e) {
      logger.error("settings", "SystemPanel::restore", "Failed to restore backup", e)
      setRestoreError(e instanceof Error ? e.message : String(e))
    } finally {
      setRestoringId(null)
      setPendingRestore(null)
    }
  }

  function formatTs(ts: string): string {
    // Filename timestamp "2026-04-17T10-30-45-123" → "2026-04-17 10:30:45"
    const m = ts.match(/^(\d{4}-\d{2}-\d{2})T(\d{2})-(\d{2})-(\d{2})/)
    if (!m) return ts
    return `${m[1]} ${m[2]}:${m[3]}:${m[4]}`
  }

  if (!loaded) return null

  return (
    <div className="space-y-6">
      {/* Autostart */}
      <div
        className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors cursor-pointer"
        onClick={toggleAutostart}
      >
        <div className="space-y-0.5">
          <div className="text-sm font-medium">{t("settings.systemAutostart")}</div>
          <div className="text-xs text-muted-foreground">{t("settings.systemAutostartDesc")}</div>
        </div>
        <Switch checked={autostart} onCheckedChange={toggleAutostart} />
      </div>

      {/* Config Backups */}
      <div className="space-y-2">
        <div className="flex items-center justify-between px-3">
          <div className="flex items-center gap-2">
            <History className="h-4 w-4 text-muted-foreground" />
            <div>
              <div className="text-sm font-medium">{t("settings.configBackupsTitle")}</div>
              <div className="text-xs text-muted-foreground">
                {t("settings.configBackupsDesc")}
              </div>
            </div>
          </div>
          <IconTip label={t("common.refresh")}>
            <Button
              variant="ghost"
              size="icon"
              onClick={loadBackups}
              disabled={backupsLoading}
              className="h-8 w-8"
            >
              <RefreshCw className={cn("h-4 w-4", backupsLoading && "animate-spin")} />
            </Button>
          </IconTip>
        </div>

        {restoreError && (
          <div className="mx-3 px-3 py-2 rounded-md bg-destructive/10 text-destructive text-xs flex items-center justify-between">
            <span className="truncate">{t("settings.configBackupsRestoreFailed", { msg: restoreError })}</span>
            <Button
              variant="ghost"
              size="icon"
              onClick={() => setRestoreError(null)}
              className="ml-2 h-6 w-6 shrink-0 text-destructive/70 hover:bg-destructive/15 hover:text-destructive"
            >
              ×
            </Button>
          </div>
        )}

        <div className="rounded-lg border bg-card overflow-hidden">
          {backups.length === 0 ? (
            <div className="px-4 py-6 text-xs text-center text-muted-foreground">
              {backupsLoading
                ? t("settings.configBackupsLoading")
                : t("settings.configBackupsEmpty")}
            </div>
          ) : (
            <div className="max-h-80 overflow-y-auto divide-y">
              {backups.map((entry) => (
                <div
                  key={entry.id}
                  className="flex items-center justify-between px-3 py-2 hover:bg-secondary/40 transition-colors"
                >
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2 text-xs">
                      <span className="font-mono text-muted-foreground">
                        {formatTs(entry.timestamp)}
                      </span>
                      <span className="inline-flex items-center px-1.5 py-0.5 rounded bg-secondary/70 text-[10px] font-medium">
                        {entry.kind}
                      </span>
                      {entry.category !== "unknown" && (
                        <span className="truncate text-foreground/80">{entry.category}</span>
                      )}
                    </div>
                    <div className="mt-0.5 text-[10px] text-muted-foreground">
                      {t("settings.configBackupsSource", { source: entry.source })}
                    </div>
                  </div>
                  <IconTip label={t("settings.configBackupsRestoreTip")}>
                    <Button
                      variant="ghost"
                      size="icon"
                      onClick={() => setPendingRestore(entry)}
                      disabled={restoringId !== null}
                      className="h-8 w-8 shrink-0"
                    >
                      {restoringId === entry.id ? (
                        <Loader2 className="h-4 w-4 animate-spin" />
                      ) : (
                        <RotateCcw className="h-4 w-4" />
                      )}
                    </Button>
                  </IconTip>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>

      <AlertDialog
        open={pendingRestore !== null}
        onOpenChange={(open) => !open && setPendingRestore(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("settings.configBackupsRestoreTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("settings.configBackupsRestoreDesc", {
                timestamp: pendingRestore ? formatTs(pendingRestore.timestamp) : "",
                kind: pendingRestore?.kind ?? "",
                category: pendingRestore?.category ?? "",
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction onClick={handleRestore}>
              {t("settings.configBackupsRestoreConfirm")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
