import { useCallback, useEffect, useState } from "react"
import {
  ChevronUp,
  Folder,
  FolderOpen,
  FolderPlus,
  Home,
  Loader2,
  RefreshCw,
  ShieldAlert,
} from "lucide-react"
import { useTranslation } from "react-i18next"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"
import { TRANSPORT_EVENT_RESYNC_REQUIRED, type DirListing } from "@/lib/transport"
import { useTransport } from "@/lib/transport-provider"
import { readFilesystemConfig } from "@/lib/filesystemConfig"
import { logger } from "@/lib/logger"

interface ServerDirectoryBrowserProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  initialPath?: string | null
  onSelect: (path: string) => void
  allowCreate?: boolean
}

export default function ServerDirectoryBrowser({
  open,
  onOpenChange,
  initialPath,
  onSelect,
  allowCreate = false,
}: ServerDirectoryBrowserProps) {
  const { t } = useTranslation()
  const transport = useTransport()
  const workspaceIsLocal = transport.fileRuntime().workspaceHost === "local"
  const [listing, setListing] = useState<DirListing | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [manualPath, setManualPath] = useState("")
  const [newFolderName, setNewFolderName] = useState("")
  const [creating, setCreating] = useState(false)
  const [createError, setCreateError] = useState<string | null>(null)
  const [createErrorDetail, setCreateErrorDetail] = useState<string | null>(null)
  const [remoteWritesAllowed, setRemoteWritesAllowed] = useState<boolean | null>(null)
  const [remoteWritesCheckFailed, setRemoteWritesCheckFailed] = useState(false)
  const [remoteWritesRefreshRevision, setRemoteWritesRefreshRevision] = useState(0)

  const load = useCallback(
    async (path?: string): Promise<DirListing | null> => {
      setLoading(true)
      setError(null)
      setCreateError(null)
      setCreateErrorDetail(null)
      try {
        const result = await transport.listServerDirectory(path)
        setListing(result)
        setManualPath(result.path)
        return result
      } catch (e) {
        const message = e instanceof Error ? e.message : String(e)
        logger.error("chat", "ServerDirectoryBrowser::load", "Failed to list server directory", e)
        setError(message)
        return null
      } finally {
        setLoading(false)
      }
    },
    [transport],
  )

  useEffect(() => {
    if (!open || !allowCreate || workspaceIsLocal) return

    let cancelled = false
    let requestRevision = 0
    const refresh = async () => {
      const currentRevision = ++requestRevision
      setRemoteWritesAllowed(null)
      setRemoteWritesCheckFailed(false)
      try {
        const config = await readFilesystemConfig(transport)
        if (!cancelled && currentRevision === requestRevision) {
          setRemoteWritesAllowed(config.allowRemoteWrites)
        }
      } catch (e) {
        if (!cancelled && currentRevision === requestRevision) {
          // Keep capability-read failures distinct from an authoritative
          // policy denial so the UI never tells users to enable an option
          // that may already be enabled. Creation stays fail-closed until a
          // retry succeeds; the backend remains the execution authority.
          setRemoteWritesCheckFailed(true)
          logger.warn(
            "chat",
            "ServerDirectoryBrowser::remoteWritesCapability",
            "Failed to read remote-write capability",
            e,
          )
        }
      }
    }

    void refresh()
    const unlistenConfig = transport.listen("config:changed", () => void refresh())
    const unlistenResync = transport.listen(TRANSPORT_EVENT_RESYNC_REQUIRED, () => void refresh())
    return () => {
      cancelled = true
      unlistenConfig()
      unlistenResync()
    }
  }, [allowCreate, open, remoteWritesRefreshRevision, transport, workspaceIsLocal])

  useEffect(() => {
    if (!open) return
    const seed = initialPath && initialPath.trim().length > 0 ? initialPath : undefined
    load(seed)
  }, [open, initialPath, load])

  const handleEnter = (path: string) => {
    if (loading) return
    load(path)
  }

  const handleManualSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    const trimmed = manualPath.trim()
    if (!trimmed) return
    load(trimmed)
  }

  const handleCreateSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!allowCreate || !listing || loading || creating) return

    const name = newFolderName.trim()
    if (!name) return
    if (name === "." || name === ".." || /[\\/]/.test(name)) {
      setCreateError(t("chat.workingDir.invalid"))
      setCreateErrorDetail(null)
      return
    }

    setCreating(true)
    setCreateError(null)
    setCreateErrorDetail(null)
    try {
      const created = await transport.createDirectory(joinDirectoryPath(listing.path, name))
      setListing(created)
      setManualPath(created.path)
      setNewFolderName("")
      onSelect(created.path)
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error(
        "chat",
        "ServerDirectoryBrowser::createDirectory",
        "Failed to create directory",
        e,
      )
      if (isRemoteWritesDisabledError(message)) {
        setRemoteWritesAllowed(false)
        setRemoteWritesCheckFailed(false)
        setCreateError(t("fileEditor.remoteWritesTitle"))
      } else if (isLocationNotWritableError(message)) {
        setCreateError(t("chat.workingDir.locationNotWritable"))
        setCreateErrorDetail(message)
      } else {
        setCreateError(message)
      }
    } finally {
      setCreating(false)
    }
  }

  const handleSelectCurrent = async () => {
    if (!listing) return
    const trimmed = manualPath.trim()
    if (trimmed && trimmed !== listing.path) {
      const result = await load(trimmed)
      if (!result) return
      onSelect(result.path)
      return
    }
    onSelect(listing.path)
  }

  const remoteWritesChecking =
    allowCreate && !workspaceIsLocal && remoteWritesAllowed === null && !remoteWritesCheckFailed
  const remoteWritesUnavailable = allowCreate && !workspaceIsLocal && remoteWritesCheckFailed
  const remoteWritesBlocked =
    allowCreate && !workspaceIsLocal && remoteWritesAllowed === false && !remoteWritesCheckFailed
  const canCreate =
    allowCreate && (workspaceIsLocal || (remoteWritesAllowed === true && !remoteWritesCheckFailed))

  const openServerSettings = () => {
    onOpenChange(false)
    window.dispatchEvent(new CustomEvent("settings:navigate", { detail: { section: "server" } }))
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>{t("chat.workingDir.browserTitle")}</DialogTitle>
          <DialogDescription>{t("chat.workingDir.browserDescription")}</DialogDescription>
        </DialogHeader>

        <form onSubmit={handleManualSubmit} className="flex items-center gap-2">
          <Button
            type="button"
            variant="ghost"
            size="icon"
            disabled={loading || !listing?.parent}
            onClick={() => listing?.parent && load(listing.parent)}
            aria-label={t("chat.workingDir.parent")}
          >
            <ChevronUp className="h-4 w-4" />
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="icon"
            disabled={loading}
            onClick={() => void load()}
            aria-label={t("chat.workingDir.home")}
          >
            <Home className="h-4 w-4" />
          </Button>
          <Input
            value={manualPath}
            onChange={(e) => setManualPath(e.target.value)}
            placeholder={t("chat.workingDir.pathPlaceholder")}
            className="flex-1 font-mono text-xs"
          />
          <Button
            type="submit"
            variant="outline"
            size="sm"
            disabled={loading}
            aria-label={t("chat.workingDir.goToPath", "跳转到路径")}
          >
            {loading ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <RefreshCw className="h-4 w-4" />
            )}
          </Button>
        </form>

        {remoteWritesChecking && (
          <div className="flex items-center gap-2 rounded-md border border-border bg-muted/20 px-3 py-3 text-xs text-muted-foreground">
            <Loader2 className="h-4 w-4 shrink-0 animate-spin" />
            {t("chat.workingDir.loading")}
          </div>
        )}

        {remoteWritesBlocked && (
          <div className="rounded-md border border-amber-500/30 bg-amber-500/5 p-3">
            <div className="flex items-start gap-2.5">
              <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0 text-amber-600" />
              <div className="min-w-0 flex-1">
                <p className="text-xs font-medium text-foreground">
                  {t("fileEditor.remoteWritesTitle")}
                </p>
                <p className="mt-1 text-xs leading-5 text-muted-foreground">
                  {t("fileEditor.remoteWritesBody")}
                </p>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="mt-2 h-8"
                  onClick={openServerSettings}
                >
                  {t("fileEditor.goToServerSettings")}
                </Button>
              </div>
            </div>
          </div>
        )}

        {remoteWritesUnavailable && (
          <div className="rounded-md border border-amber-500/30 bg-amber-500/5 p-3">
            <div className="flex items-start gap-2.5">
              <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0 text-amber-600" />
              <div className="min-w-0 flex-1">
                <p className="text-xs font-medium text-foreground">
                  {t("chat.workingDir.remoteWritesUnavailable")}
                </p>
                <p className="mt-1 text-xs leading-5 text-muted-foreground">
                  {t("chat.workingDir.remoteWritesUnavailableBody")}
                </p>
                <div className="mt-2 flex flex-wrap gap-2">
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="h-8"
                    onClick={() => setRemoteWritesRefreshRevision((revision) => revision + 1)}
                  >
                    {t("common.retry")}
                  </Button>
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="h-8"
                    onClick={openServerSettings}
                  >
                    {t("fileEditor.goToServerSettings")}
                  </Button>
                </div>
              </div>
            </div>
          </div>
        )}

        {canCreate && (
          <form
            onSubmit={handleCreateSubmit}
            className="rounded-md border border-border bg-muted/20 p-3"
          >
            <div className="mb-2 flex items-center gap-2 text-xs font-medium">
              <FolderPlus className="h-3.5 w-3.5 text-primary" />
              {t("fileBrowser.newFolder", { defaultValue: "New folder" })}
            </div>
            <div className="flex items-center gap-2">
              <Input
                value={newFolderName}
                onChange={(e) => setNewFolderName(e.target.value)}
                placeholder={t("knowledge.folderNamePlaceholder", {
                  defaultValue: "Folder name",
                })}
                disabled={!listing || loading || creating}
                className="h-9 flex-1 text-sm"
              />
              <Button
                type="submit"
                size="sm"
                disabled={!listing || loading || creating || !newFolderName.trim()}
              >
                {creating ? <Loader2 className="h-4 w-4 animate-spin" /> : t("common.create")}
              </Button>
            </div>
            {createError && (
              <div className="mt-2 text-xs text-destructive">
                <p>{createError}</p>
                {createErrorDetail && (
                  <details className="mt-1 text-muted-foreground">
                    <summary className="cursor-pointer select-none">{t("chat.details")}</summary>
                    <p className="mt-1 break-all font-mono text-[11px]">{createErrorDetail}</p>
                  </details>
                )}
              </div>
            )}
          </form>
        )}

        <div className="rounded border border-border bg-muted/20 h-[360px] overflow-y-auto">
          {loading && (
            <div className="flex items-center justify-center h-full text-xs text-muted-foreground">
              <Loader2 className="h-4 w-4 animate-spin mr-2" />
              {t("chat.workingDir.loading")}
            </div>
          )}
          {!loading && error && (
            <div className="flex flex-col items-center justify-center h-full text-xs text-destructive px-4 text-center">
              <p className="font-medium mb-1">{t("chat.workingDir.loadError")}</p>
              <p className="break-all">{error}</p>
            </div>
          )}
          {!loading && !error && listing && listing.entries.length === 0 && (
            <div className="flex items-center justify-center h-full text-xs text-muted-foreground">
              {t("chat.workingDir.empty")}
            </div>
          )}
          {!loading && !error && listing && listing.entries.length > 0 && (
            <ul className="divide-y divide-border">
              {listing.entries.map((entry) => (
                <li key={entry.path}>
                  <button
                    type="button"
                    disabled={!entry.isDir}
                    onClick={() => entry.isDir && handleEnter(entry.path)}
                    className={cn(
                      "w-full flex items-center gap-2 px-3 py-1.5 text-xs hover:bg-muted transition-colors text-left",
                      !entry.isDir && "opacity-50 cursor-default",
                    )}
                  >
                    {entry.isDir ? (
                      <Folder className="h-3.5 w-3.5 shrink-0 text-primary" />
                    ) : (
                      <FolderOpen className="h-3.5 w-3.5 shrink-0 text-muted-foreground/60 invisible" />
                    )}
                    <span className="truncate font-mono">{entry.name}</span>
                    {!entry.isDir && (
                      <span className="ml-auto text-muted-foreground">
                        {t("chat.workingDir.fileLabel")}
                      </span>
                    )}
                  </button>
                </li>
              ))}
            </ul>
          )}
          {!loading && !error && listing?.truncated && (
            <div className="border-t border-border px-3 py-2 text-[11px] text-muted-foreground">
              {t("chat.workingDir.truncated")}
            </div>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button onClick={handleSelectCurrent} disabled={!listing || loading}>
            {t("chat.workingDir.selectCurrent")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function joinDirectoryPath(base: string, name: string): string {
  const separator = base.includes("\\") && !base.includes("/") ? "\\" : "/"
  const normalizedBase = base.replace(/[\\/]+$/, "")
  if (!normalizedBase) return `${separator}${name}`
  return `${normalizedBase}${separator}${name}`
}

function isRemoteWritesDisabledError(message: string): boolean {
  return /allowremotewrites|remote file writes are disabled/i.test(message)
}

function isLocationNotWritableError(message: string): boolean {
  return /directory is not writable|permission denied|operation not permitted|access is denied|read-only file system|os error (?:5|13|30)/i.test(
    message,
  )
}
