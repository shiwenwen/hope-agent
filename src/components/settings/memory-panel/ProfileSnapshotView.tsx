import { useCallback, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { Button } from "@/components/ui/button"
import { Loader2, Sparkles, UserCircle } from "lucide-react"

// Mirrors ha-core `ProfileSnapshotRecord` (camelCase).
interface ProfileSnapshotRecord {
  scopeType: string
  scopeId?: string | null
  version: number
  bodyMd: string
  sourceRunId: string
  createdAt: string
}

// Mirrors ha-core `ProfileReport` (camelCase).
interface ProfileReport {
  runId?: string | null
  scanned: number
  scopes: number
  snapshotsWritten: number
  durationMs: number
  note?: string | null
}

const scopeLabel = (r: { scopeType: string; scopeId?: string | null }) =>
  r.scopeType === "global" ? "global" : `${r.scopeType}:${r.scopeId ?? "?"}`

/**
 * Read-only Memory Profile view (next-gen Dreaming Phase 4). Shows the latest
 * synthesised profile snapshot per scope (global / agent / project) via
 * `dreaming_list_profile_snapshots`, and a manual "refresh" that runs an
 * LLM-rewrite synthesis cycle (`dreaming_run_profile`). The profile is
 * grounded in active claims — editing / rejecting lands with the correction
 * loop in a later PR.
 */
export default function ProfileSnapshotView() {
  const { t } = useTranslation()
  const [snapshots, setSnapshots] = useState<ProfileSnapshotRecord[]>([])
  const [loading, setLoading] = useState(false)
  const [refreshing, setRefreshing] = useState(false)

  const load = useCallback(async () => {
    setLoading(true)
    try {
      const list = await getTransport().call<ProfileSnapshotRecord[]>(
        "dreaming_list_profile_snapshots",
      )
      setSnapshots(list ?? [])
    } catch (e) {
      logger.error("settings", "ProfileSnapshotView::list", "Failed to list snapshots", e)
      setSnapshots([])
    } finally {
      setLoading(false)
    }
  }, [])

  const refresh = useCallback(async () => {
    setRefreshing(true)
    try {
      const r = await getTransport().call<ProfileReport>("dreaming_run_profile")
      await load()
      if (r?.runId) {
        toast.success(
          t("settings.profile.refreshDone", { count: r?.snapshotsWritten ?? 0 }),
        )
      } else {
        // Skipped before a run row was created (disabled / lock contention).
        toast.message(t("settings.profile.refreshSkipped"), {
          description: r?.note ?? undefined,
        })
      }
    } catch (e) {
      logger.error("settings", "ProfileSnapshotView::refresh", "Failed to run synthesis", e)
      toast.error(t("settings.profile.refreshFailed"))
    } finally {
      setRefreshing(false)
    }
  }, [t, load])

  useEffect(() => {
    void load()
  }, [load])

  useEffect(() => {
    return getTransport().listen("dreaming:cycle_complete", (raw) => {
      const payload = raw as { phase?: string }
      if (payload.phase === "profile") void load()
    })
  }, [load])

  return (
    <div className="flex-1 overflow-y-auto p-6 space-y-3">
      <div className="flex items-center justify-between gap-2">
        <div>
          <div className="text-sm font-medium">{t("settings.profile.title")}</div>
          <div className="text-xs text-muted-foreground">{t("settings.profile.desc")}</div>
        </div>
        <Button
          variant="outline"
          size="sm"
          className="h-8 gap-1.5 text-xs"
          onClick={refresh}
          disabled={refreshing}
        >
          {refreshing ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <Sparkles className="h-3.5 w-3.5" />
          )}
          {refreshing ? t("settings.profile.refreshing") : t("settings.profile.refresh")}
        </Button>
      </div>

      {loading ? (
        <div className="px-3 py-10 text-xs text-muted-foreground text-center inline-flex items-center gap-1.5 w-full justify-center">
          <Loader2 className="h-3.5 w-3.5 animate-spin" />
          {t("common.loading")}
        </div>
      ) : snapshots.length === 0 ? (
        <div className="rounded-lg border border-border/60 px-4 py-10 text-center text-xs text-muted-foreground">
          <UserCircle className="h-6 w-6 mx-auto mb-2 opacity-40" />
          <div>{t("settings.profile.empty")}</div>
          <div className="mt-1">{t("settings.profile.emptyHint")}</div>
        </div>
      ) : (
        <div className="space-y-3">
          {snapshots.map((s) => (
            <div
              key={`${s.scopeType}:${s.scopeId ?? ""}`}
              className="border border-border/60 rounded-lg overflow-hidden"
            >
              <div className="px-3 py-2 border-b border-border/60 bg-secondary/20 flex items-center justify-between gap-2">
                <span className="text-xs font-medium font-mono">{scopeLabel(s)}</span>
                <span className="text-[10px] text-muted-foreground">
                  {t("settings.profile.version", { version: s.version })} · {s.createdAt}
                </span>
              </div>
              <pre className="px-3 py-2 text-xs whitespace-pre-wrap break-words font-sans text-foreground/90">
                {s.bodyMd.trim()}
              </pre>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
