import { useCallback, useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { AlertTriangle, Loader2, Power, RefreshCw, RotateCcw, ShieldAlert } from "lucide-react"
import { Button } from "@/components/ui/button"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"
import { backupMetadataLabel } from "./backupMetadataLabel"

export interface ConfigHealth {
  ok: boolean
  status: string
  path?: string | null
  error?: string | null
  message?: string | null
}

interface AutosaveEntry {
  id: string
  timestamp: string
  kind: string
  category: string
  source: string
}

interface ConfigRecoveryScreenProps {
  health: ConfigHealth | null
  onRecovered: () => Promise<void>
}

function formatTs(ts: string): string {
  const m = ts.match(/^(\d{4}-\d{2}-\d{2})T(\d{2})-(\d{2})-(\d{2})/)
  if (!m) return ts
  return `${m[1]} ${m[2]}:${m[3]}:${m[4]}`
}

function errorMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e)
}

export default function ConfigRecoveryScreen({ health, onRecovered }: ConfigRecoveryScreenProps) {
  const { t } = useTranslation()
  const [backups, setBackups] = useState<AutosaveEntry[]>([])
  const [loadingBackups, setLoadingBackups] = useState(false)
  const [retrying, setRetrying] = useState(false)
  const [restoringId, setRestoringId] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  const configBackups = useMemo(
    () => backups.filter((entry) => entry.kind === "config"),
    [backups],
  )

  const loadBackups = useCallback(async () => {
    setLoadingBackups(true)
    try {
      const list = await getTransport().call<AutosaveEntry[]>("list_settings_backups_cmd")
      setBackups(list)
    } catch (e) {
      logger.error("config", "ConfigRecoveryScreen::loadBackups", "Failed to list backups", e)
      setError(errorMessage(e))
    } finally {
      setLoadingBackups(false)
    }
  }, [])

  useEffect(() => {
    loadBackups()
  }, [loadBackups])

  async function retryRead() {
    setRetrying(true)
    setError(null)
    try {
      await onRecovered()
    } catch (e) {
      setError(errorMessage(e))
    } finally {
      setRetrying(false)
    }
  }

  async function restore(entry: AutosaveEntry) {
    setRestoringId(entry.id)
    setError(null)
    try {
      await getTransport().call("restore_settings_backup_cmd", { id: entry.id })
      await onRecovered()
    } catch (e) {
      logger.error("config", "ConfigRecoveryScreen::restore", "Failed to restore config backup", e)
      setError(errorMessage(e))
      await loadBackups()
    } finally {
      setRestoringId(null)
    }
  }

  async function restart() {
    setError(null)
    try {
      const result = await getTransport().call<{ ok?: boolean; note?: string } | undefined>(
        "request_app_restart",
      )
      if (result && result.ok === false) {
        setError(result.note || t("configRecovery.restartFailed"))
      }
    } catch (e) {
      logger.error("config", "ConfigRecoveryScreen::restart", "Failed to request restart", e)
      setError(t("configRecovery.restartFailed"))
    }
  }

  return (
    <div className="relative z-10 flex min-h-screen items-center justify-center px-4 py-8">
      <div className="w-full max-w-4xl overflow-hidden rounded-lg border border-border bg-background/95 shadow-xl">
        <div className="border-b border-border px-5 py-4">
          <div className="flex items-start gap-3">
            <div className="mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-md bg-destructive/10 text-destructive">
              <ShieldAlert className="h-5 w-5" />
            </div>
            <div className="min-w-0 flex-1">
              <h1 className="text-xl font-semibold tracking-normal text-foreground">
                {t("configRecovery.title")}
              </h1>
              <p className="mt-1 max-w-3xl text-sm leading-6 text-muted-foreground">
                {t("configRecovery.subtitle")}
              </p>
            </div>
          </div>
        </div>

        <div className="grid gap-5 p-5 lg:grid-cols-[minmax(0,1fr)_minmax(320px,420px)]">
          <div className="space-y-4">
            <div className="rounded-md border border-border bg-muted/30 p-4">
              <div className="space-y-3 text-sm">
                <div>
                  <div className="text-xs font-medium uppercase text-muted-foreground">
                    {t("configRecovery.path")}
                  </div>
                  <div className="mt-1 break-all font-mono text-xs text-foreground">
                    {health?.path || "config.json"}
                  </div>
                </div>
                <div>
                  <div className="text-xs font-medium uppercase text-muted-foreground">
                    {t("configRecovery.error")}
                  </div>
                  <div className="mt-1 break-words text-destructive">
                    {health?.error || health?.status || t("common.unknown")}
                  </div>
                </div>
                {health?.message && (
                  <div>
                    <div className="text-xs font-medium uppercase text-muted-foreground">
                      {t("configRecovery.details")}
                    </div>
                    <div className="mt-1 break-words text-muted-foreground">
                      {health.message}
                    </div>
                  </div>
                )}
              </div>
            </div>

            {error && (
              <div className="flex items-start gap-2 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
                <span className="min-w-0 break-words">{error}</span>
              </div>
            )}

            <div className="flex flex-wrap gap-2">
              <Button onClick={retryRead} disabled={retrying || restoringId !== null}>
                {retrying ? (
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                ) : (
                  <RefreshCw className="mr-2 h-4 w-4" />
                )}
                {retrying ? t("configRecovery.retrying") : t("configRecovery.retry")}
              </Button>
              <Button variant="outline" onClick={restart} disabled={retrying || restoringId !== null}>
                <Power className="mr-2 h-4 w-4" />
                {t("configRecovery.restart")}
              </Button>
            </div>
          </div>

          <div className="rounded-md border border-border bg-card">
            <div className="border-b border-border px-4 py-3">
              <div className="text-sm font-medium text-foreground">
                {t("configRecovery.backupsTitle")}
              </div>
              <div className="mt-1 text-xs leading-5 text-muted-foreground">
                {t("configRecovery.backupsDesc")}
              </div>
            </div>

            {loadingBackups ? (
              <div className="flex items-center justify-center gap-2 px-4 py-10 text-sm text-muted-foreground">
                <Loader2 className="h-4 w-4 animate-spin" />
                {t("configRecovery.loading")}
              </div>
            ) : configBackups.length === 0 ? (
              <div className="px-4 py-10 text-center">
                <div className="text-sm font-medium text-foreground">
                  {t("configRecovery.empty")}
                </div>
                <div className="mx-auto mt-2 max-w-sm text-xs leading-5 text-muted-foreground">
                  {t("configRecovery.emptyHint")}
                </div>
              </div>
            ) : (
              <div className="max-h-96 divide-y divide-border overflow-y-auto">
                {configBackups.map((entry) => {
                  const busy = restoringId === entry.id
                  return (
                    <div
                      key={entry.id}
                      className={cn(
                        "flex items-center gap-3 px-4 py-3 transition-colors",
                        busy ? "bg-secondary/60" : "hover:bg-secondary/40",
                      )}
                    >
                      <div className="min-w-0 flex-1">
                        <div className="truncate font-mono text-xs text-foreground">
                          {formatTs(entry.timestamp)}
                        </div>
                        <div className="mt-1 flex min-w-0 items-center gap-2 text-[11px] text-muted-foreground">
                          <span className="rounded bg-secondary px-1.5 py-0.5 font-medium">
                            {backupMetadataLabel(t, "category", entry.category)}
                          </span>
                          <span className="truncate">
                            {t("configRecovery.source")}: {backupMetadataLabel(t, "source", entry.source)}
                          </span>
                        </div>
                      </div>
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => restore(entry)}
                        disabled={restoringId !== null || retrying}
                      >
                        {busy ? (
                          <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                        ) : (
                          <RotateCcw className="mr-2 h-4 w-4" />
                        )}
                        {busy ? t("configRecovery.restoring") : t("configRecovery.restore")}
                      </Button>
                    </div>
                  )
                })}
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}
