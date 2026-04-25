/**
 * MCP Servers settings panel.
 *
 * List + CRUD for the servers persisted in `AppConfig.mcp_servers`. Calls
 * go through `@/lib/mcp.ts`, which unifies the Tauri IPC and HTTP
 * transports. Status dots come from the live `McpServerStatusSnapshot`
 * joined on the list response.
 */

import { useCallback, useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  Plug,
  Plus,
  Upload,
  RefreshCw,
  Loader2,
  Trash2,
  CheckCircle2,
  AlertCircle,
  Link2,
  KeyRound,
  LogOut,
} from "lucide-react"

import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
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
import { logger } from "@/lib/logger"
import { toast } from "sonner"
import { getTransport } from "@/lib/transport-provider"
import {
  listServers,
  removeServer,
  reconnectServer,
  testConnection,
  startOauth,
  signOut,
  MCP_EVENTS,
  type McpServerSummary,
  type McpServerState,
  type McpTransportKind,
} from "@/lib/mcp"
import McpServerEditDialog from "./McpServerEditDialog"
import McpImportDialog from "./McpImportDialog"

/** Single slot describing what the edit dialog is showing (if anything).
 * Combining "add" vs "edit(existing server)" into one discriminator
 * removes a whole class of bugs where `editingId` points at a row that
 * was deleted between refresh ticks. */
type EditTarget =
  | { mode: "add" }
  | { mode: "edit"; server: McpServerSummary }
  | null

// ── Status visuals ───────────────────────────────────────────────

const STATE_DOT_CLASS: Record<McpServerState, string> = {
  ready: "bg-green-500",
  connecting: "bg-yellow-500 animate-pulse",
  needsAuth: "bg-yellow-500",
  failed: "bg-red-500",
  idle: "bg-muted-foreground/50",
  disabled: "bg-muted-foreground/30",
}

const TRANSPORT_BADGE: Record<McpTransportKind, string> = {
  stdio: "bg-blue-500/15 text-blue-600 dark:text-blue-400",
  streamableHttp: "bg-purple-500/15 text-purple-600 dark:text-purple-400",
  sse: "bg-orange-500/15 text-orange-600 dark:text-orange-400",
  websocket: "bg-emerald-500/15 text-emerald-600 dark:text-emerald-400",
}

function transportKindOf(s: McpServerSummary): McpTransportKind {
  return s.transport.kind
}

// ── Main panel ───────────────────────────────────────────────────

export default function McpServersPanel() {
  const { t } = useTranslation()
  const [servers, setServers] = useState<McpServerSummary[]>([])
  const [loading, setLoading] = useState(true)
  const [edit, setEdit] = useState<EditTarget>(null)
  const [importing, setImporting] = useState(false)
  const [busyId, setBusyId] = useState<string | null>(null)
  const [pendingDelete, setPendingDelete] =
    useState<{ id: string; name: string } | null>(null)

  const refresh = useCallback(async () => {
    try {
      const next = await listServers()
      setServers(next)
    } catch (e) {
      logger.error("mcp", "McpServersPanel::refresh", "Failed to load servers", e)
      toast.error(t("settings.mcp.loadFailed"))
    } finally {
      setLoading(false)
    }
  }, [t])

  // Trailing-edge debounce for refresh: backend emits SERVER_STATUS_CHANGED
  // on every state transition (Connecting → Ready fires ≥ 2 events) and
  // SERVERS_CHANGED on every config mutation — without coalescing, a
  // 5-server eager-connect burst causes ~10 listServers round-trips in
  // under a second.
  const refreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const scheduleRefresh = useCallback(() => {
    if (refreshTimerRef.current !== null) clearTimeout(refreshTimerRef.current)
    refreshTimerRef.current = setTimeout(() => {
      refreshTimerRef.current = null
      refresh()
    }, 150)
  }, [refresh])

  useEffect(() => {
    refresh()
  }, [refresh])

  useEffect(() => {
    const transport = getTransport()
    const cleanups = [
      transport.listen(MCP_EVENTS.SERVERS_CHANGED, scheduleRefresh),
      transport.listen(MCP_EVENTS.SERVER_STATUS_CHANGED, scheduleRefresh),
      transport.listen(MCP_EVENTS.AUTH_REQUIRED, (payload) => {
        if (!payload || typeof payload !== "object") return
        const event = payload as { name?: unknown; authUrl?: unknown }
        if (typeof event.name !== "string" || typeof event.authUrl !== "string") return
        toast.info(t("settings.mcp.authRequired", { name: event.name }), {
          description: event.authUrl,
          duration: 15000,
        })
      }),
      // AUTH_COMPLETED only surfaces a toast — SERVER_STATUS_CHANGED is
      // what triggers the actual refresh, so don't re-pull here.
      transport.listen(MCP_EVENTS.AUTH_COMPLETED, (payload) => {
        if (!payload || typeof payload !== "object") return
        const event = payload as { name?: unknown; ok?: unknown; error?: unknown }
        if (typeof event.name !== "string" || typeof event.ok !== "boolean") return
        if (event.ok) {
          toast.success(t("settings.mcp.authSuccess", { name: event.name }))
        } else {
          toast.error(
            (typeof event.error === "string" ? event.error : undefined) ??
              t("settings.mcp.authFailed", { name: event.name }),
          )
        }
      }),
    ]
    return () => {
      cleanups.forEach((fn) => fn())
      if (refreshTimerRef.current !== null) {
        clearTimeout(refreshTimerRef.current)
        refreshTimerRef.current = null
      }
    }
  }, [scheduleRefresh, t])

  const runBusy = useCallback(
    async (id: string, fn: () => Promise<void>) => {
      setBusyId(id)
      try {
        await fn()
      } catch (e) {
        toast.error(String(e))
      } finally {
        setBusyId(null)
      }
    },
    [],
  )

  const handleTest = useCallback(
    (id: string) =>
      runBusy(id, async () => {
        const snap = await testConnection(id)
        if (snap.state === "ready") {
          toast.success(
            t("settings.mcp.testSuccess", { count: snap.toolCount }),
          )
        } else {
          toast.error(snap.reason ?? t("settings.mcp.testFailed"))
        }
        scheduleRefresh()
      }),
    [runBusy, scheduleRefresh, t],
  )

  const handleReconnect = useCallback(
    (id: string) =>
      runBusy(id, async () => {
        await reconnectServer(id)
        scheduleRefresh()
      }),
    [runBusy, scheduleRefresh],
  )

  const handleAuthorize = useCallback(
    (id: string) =>
      runBusy(id, async () => {
        await startOauth(id)
        toast.info(t("settings.mcp.authStarted"))
      }),
    [runBusy, t],
  )

  const handleSignOut = useCallback(
    (id: string, name: string) =>
      runBusy(id, async () => {
        await signOut(id)
        toast.success(t("settings.mcp.signOutSuccess", { name }))
        scheduleRefresh()
      }),
    [runBusy, scheduleRefresh, t],
  )

  const confirmDelete = useCallback(async () => {
    if (!pendingDelete) return
    const { id, name } = pendingDelete
    setPendingDelete(null)
    try {
      await removeServer(id)
      toast.success(t("settings.mcp.deleted", { name }))
      refresh()
    } catch (e) {
      toast.error(String(e))
    }
  }, [pendingDelete, refresh, t])

  const handleAfterEdit = useCallback(() => {
    setEdit(null)
    refresh()
  }, [refresh])

  return (
    <div className="flex flex-col h-full overflow-hidden">
      {/* Header */}
      <div className="flex items-center justify-between gap-4 px-6 py-4 border-b border-border">
        <div>
          <h2 className="text-lg font-semibold flex items-center gap-2">
            <Plug className="h-5 w-5 text-primary" />
            {t("settings.mcp.title")}
          </h2>
          <p className="text-sm text-muted-foreground mt-0.5">
            {t("settings.mcp.subtitle")}
          </p>
        </div>
        <div className="flex gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={() => setImporting(true)}
            className="gap-1.5"
          >
            <Upload className="h-3.5 w-3.5" />
            {t("settings.mcp.importJson")}
          </Button>
          <Button
            size="sm"
            onClick={() => setEdit({ mode: "add" })}
            className="gap-1.5"
          >
            <Plus className="h-3.5 w-3.5" />
            {t("settings.mcp.addServer")}
          </Button>
        </div>
      </div>

      {/* List */}
      <div className="flex-1 overflow-y-auto">
        {loading ? (
          <div className="flex items-center justify-center h-32 text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin mr-2" />
            {t("common.loading")}
          </div>
        ) : servers.length === 0 ? (
          <EmptyState
            onAdd={() => setEdit({ mode: "add" })}
            onImport={() => setImporting(true)}
          />
        ) : (
          <div className="divide-y divide-border">
            {servers.map((server) => (
              <ServerRow
                key={server.id}
                server={server}
                busy={busyId === server.id}
                onEdit={() => setEdit({ mode: "edit", server })}
                onTest={() => handleTest(server.id)}
                onReconnect={() => handleReconnect(server.id)}
                onAuthorize={() => handleAuthorize(server.id)}
                onSignOut={() => handleSignOut(server.id, server.name)}
                onDelete={() =>
                  setPendingDelete({ id: server.id, name: server.name })
                }
              />
            ))}
          </div>
        )}
      </div>

      {/* Edit / Add dialogs */}
      {edit && (
        <McpServerEditDialog
          open
          initial={edit.mode === "edit" ? edit.server : null}
          onClose={() => setEdit(null)}
          onSaved={handleAfterEdit}
        />
      )}

      {importing && (
        <McpImportDialog
          open
          onClose={() => setImporting(false)}
          onImported={() => {
            setImporting(false)
            refresh()
          }}
        />
      )}

      <AlertDialog
        open={pendingDelete !== null}
        onOpenChange={(o) => {
          if (!o) setPendingDelete(null)
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t("settings.mcp.confirmDeleteTitle", {
                name: pendingDelete?.name ?? "",
              })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t("settings.mcp.confirmDelete", {
                name: pendingDelete?.name ?? "",
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              onClick={confirmDelete}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            >
              {t("common.delete")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

// ── Row ──────────────────────────────────────────────────────────

function ServerRow({
  server,
  busy,
  onEdit,
  onTest,
  onReconnect,
  onAuthorize,
  onSignOut,
  onDelete,
}: {
  server: McpServerSummary
  busy: boolean
  onEdit: () => void
  onTest: () => void
  onReconnect: () => void
  onAuthorize: () => void
  onSignOut: () => void
  onDelete: () => void
}) {
  const { t } = useTranslation()
  const state = (server.state ?? "idle") as McpServerState
  const dot = STATE_DOT_CLASS[state]
  const transport = transportKindOf(server)
  const badge = TRANSPORT_BADGE[transport]
  const isFailed = state === "failed"
  const isReady = state === "ready"
  const isNeedsAuth = state === "needsAuth"
  // `server.oauth` is only ever set on networked transports (backend +
  // edit dialog reject it on stdio), so a simple presence check is
  // sufficient here.
  const hasOauth = Boolean(server.oauth)

  return (
    <div className="px-6 py-4 hover:bg-muted/30 transition-colors">
      <div className="flex items-start justify-between gap-4">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <IconTip label={t(`settings.mcp.state.${state}`)}>
              <span
                className={`h-2 w-2 rounded-full shrink-0 ${dot}`}
                aria-label={t(`settings.mcp.state.${state}`)}
              />
            </IconTip>
            <span className="font-medium truncate">{server.name}</span>
            <span
              className={`text-xs px-1.5 py-0.5 rounded ${badge}`}
            >
              {transport}
            </span>
            {!server.enabled && (
              <span className="text-xs text-muted-foreground">
                ({t("settings.mcp.disabled")})
              </span>
            )}
            {isReady && (
              <span className="text-xs text-muted-foreground ml-auto">
                {t("settings.mcp.toolCount", { count: server.toolCount })}
              </span>
            )}
          </div>
          {server.description && (
            <p className="text-xs text-muted-foreground mt-1 ml-4 line-clamp-1">
              {server.description}
            </p>
          )}
          {isFailed && server.reason && (
            <p className="text-xs text-destructive mt-1 ml-4 flex items-start gap-1">
              <AlertCircle className="h-3 w-3 shrink-0 mt-0.5" />
              <span className="line-clamp-2">{server.reason}</span>
            </p>
          )}
        </div>
        <div className="flex gap-1 shrink-0">
          <Button
            variant="ghost"
            size="sm"
            onClick={onTest}
            disabled={busy}
            className="h-7 px-2 gap-1"
          >
            {busy ? (
              <Loader2 className="h-3 w-3 animate-spin" />
            ) : (
              <CheckCircle2 className="h-3 w-3" />
            )}
            {t("settings.mcp.test")}
          </Button>
          {isFailed && (
            <Button
              variant="ghost"
              size="sm"
              onClick={onReconnect}
              disabled={busy}
              className="h-7 px-2 gap-1"
            >
              <RefreshCw className="h-3 w-3" />
              {t("settings.mcp.reconnect")}
            </Button>
          )}
          {hasOauth && (isNeedsAuth || isFailed) && (
            <Button
              variant="outline"
              size="sm"
              onClick={onAuthorize}
              disabled={busy}
              className="h-7 px-2 gap-1"
            >
              {busy ? (
                <Loader2 className="h-3 w-3 animate-spin" />
              ) : (
                <KeyRound className="h-3 w-3" />
              )}
              {t("settings.mcp.authorize")}
            </Button>
          )}
          {hasOauth && isReady && (
            <Button
              variant="ghost"
              size="sm"
              onClick={onSignOut}
              disabled={busy}
              className="h-7 px-2 gap-1"
            >
              <LogOut className="h-3 w-3" />
              {t("settings.mcp.signOut")}
            </Button>
          )}
          <Button
            variant="ghost"
            size="sm"
            onClick={onEdit}
            className="h-7 px-2"
          >
            {t("settings.mcp.edit")}
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={onDelete}
            className="h-7 w-7 p-0 text-destructive hover:text-destructive hover:bg-destructive/10"
          >
            <Trash2 className="h-3.5 w-3.5" />
          </Button>
        </div>
      </div>
    </div>
  )
}

// ── Empty state ──────────────────────────────────────────────────

function EmptyState({
  onAdd,
  onImport,
}: {
  onAdd: () => void
  onImport: () => void
}) {
  const { t } = useTranslation()
  return (
    <div className="flex flex-col items-center justify-center py-16 px-6 text-center">
      <Link2 className="h-10 w-10 text-muted-foreground/40 mb-3" />
      <h3 className="text-base font-medium">{t("settings.mcp.emptyTitle")}</h3>
      <p className="text-sm text-muted-foreground mt-1 max-w-md">
        {t("settings.mcp.emptyDesc")}
      </p>
      <div className="flex gap-2 mt-4">
        <Button variant="outline" onClick={onImport} className="gap-1.5">
          <Upload className="h-3.5 w-3.5" />
          {t("settings.mcp.importJson")}
        </Button>
        <Button onClick={onAdd} className="gap-1.5">
          <Plus className="h-3.5 w-3.5" />
          {t("settings.mcp.addServer")}
        </Button>
      </div>
    </div>
  )
}
