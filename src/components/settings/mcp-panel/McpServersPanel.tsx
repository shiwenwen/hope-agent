/**
 * MCP Servers settings panel.
 *
 * List + CRUD for the servers persisted in `AppConfig.mcp_servers`. Calls
 * go through `@/lib/mcp.ts`, which unifies the Tauri IPC and HTTP
 * transports. Status dots come from the live `McpServerStatusSnapshot`
 * joined on the list response.
 */

import { useCallback, useEffect, useState } from "react"
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
} from "lucide-react"

import { Button } from "@/components/ui/button"
import { logger } from "@/lib/logger"
import { toast } from "sonner"
import {
  listServers,
  removeServer,
  reconnectServer,
  testConnection,
  type McpServerSummary,
  type McpServerState,
  type McpTransportKind,
} from "@/lib/mcp"
import McpServerEditDialog from "./McpServerEditDialog"
import McpImportDialog from "./McpImportDialog"

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
  const [editingId, setEditingId] = useState<string | null>(null)
  const [addingNew, setAddingNew] = useState(false)
  const [importing, setImporting] = useState(false)
  const [busyId, setBusyId] = useState<string | null>(null)

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

  useEffect(() => {
    refresh()
  }, [refresh])

  // Subscribe to backend events so status dots / counts update live.
  useEffect(() => {
    let cleanup: (() => void) | undefined
    // The transport.listen is initialized lazily; call it via dynamic
    // import to keep the panel file independent of transport-provider's
    // init order.
    import("@/lib/transport-provider").then(({ transport }) => {
      cleanup = transport.listen("mcp:servers_changed", () => {
        refresh()
      })
    })
    return () => cleanup?.()
  }, [refresh])

  const handleTest = useCallback(
    async (id: string) => {
      setBusyId(id)
      try {
        const snap = await testConnection(id)
        if (snap.state === "ready") {
          toast.success(
            t("settings.mcp.testSuccess", { count: snap.toolCount }),
          )
        } else {
          toast.error(snap.reason ?? t("settings.mcp.testFailed"))
        }
        refresh()
      } catch (e) {
        toast.error(String(e))
      } finally {
        setBusyId(null)
      }
    },
    [refresh, t],
  )

  const handleReconnect = useCallback(
    async (id: string) => {
      setBusyId(id)
      try {
        await reconnectServer(id)
        refresh()
      } catch (e) {
        toast.error(String(e))
      } finally {
        setBusyId(null)
      }
    },
    [refresh],
  )

  const handleDelete = useCallback(
    async (id: string, name: string) => {
      // Plain confirm here — AlertDialog works but adds overhead for a
      // screen users enter rarely.
      const ok = window.confirm(
        t("settings.mcp.confirmDelete", { name }),
      )
      if (!ok) return
      try {
        await removeServer(id)
        toast.success(t("settings.mcp.deleted", { name }))
        refresh()
      } catch (e) {
        toast.error(String(e))
      }
    },
    [refresh, t],
  )

  const handleAfterEdit = useCallback(() => {
    setEditingId(null)
    setAddingNew(false)
    refresh()
  }, [refresh])

  const editing = editingId
    ? servers.find((s) => s.id === editingId) ?? null
    : null

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
            onClick={() => setAddingNew(true)}
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
          <EmptyState onAdd={() => setAddingNew(true)} onImport={() => setImporting(true)} />
        ) : (
          <div className="divide-y divide-border">
            {servers.map((server) => (
              <ServerRow
                key={server.id}
                server={server}
                busy={busyId === server.id}
                onEdit={() => setEditingId(server.id)}
                onTest={() => handleTest(server.id)}
                onReconnect={() => handleReconnect(server.id)}
                onDelete={() => handleDelete(server.id, server.name)}
              />
            ))}
          </div>
        )}
      </div>

      {/* Edit / Add dialogs */}
      {(editing || addingNew) && (
        <McpServerEditDialog
          open
          initial={editing}
          onClose={() => {
            setEditingId(null)
            setAddingNew(false)
          }}
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
  onDelete,
}: {
  server: McpServerSummary
  busy: boolean
  onEdit: () => void
  onTest: () => void
  onReconnect: () => void
  onDelete: () => void
}) {
  const { t } = useTranslation()
  const state = (server.state ?? "idle") as McpServerState
  const dot = STATE_DOT_CLASS[state]
  const transport = transportKindOf(server)
  const badge = TRANSPORT_BADGE[transport]
  const isFailed = state === "failed"
  const isReady = state === "ready"

  return (
    <div className="px-6 py-4 hover:bg-muted/30 transition-colors">
      <div className="flex items-start justify-between gap-4">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <div
              className={`h-2 w-2 rounded-full shrink-0 ${dot}`}
              title={t(`settings.mcp.state.${state}`)}
            />
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
