import { useCallback, useEffect, useState } from "react"
import { ChevronUp, Folder, FolderOpen, Loader2, RefreshCw } from "lucide-react"
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
import { type DirListing } from "@/lib/transport"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"

interface ServerDirectoryBrowserProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  initialPath?: string | null
  onSelect: (path: string) => void
}

export default function ServerDirectoryBrowser({
  open,
  onOpenChange,
  initialPath,
  onSelect,
}: ServerDirectoryBrowserProps) {
  const { t } = useTranslation()
  const [listing, setListing] = useState<DirListing | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [manualPath, setManualPath] = useState("")

  const load = useCallback(async (path?: string) => {
    setLoading(true)
    setError(null)
    try {
      const result = await getTransport().listServerDirectory(path)
      setListing(result)
      setManualPath(result.path)
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.error(
        "chat",
        "ServerDirectoryBrowser::load",
        "Failed to list server directory",
        e,
      )
      setError(message)
    } finally {
      setLoading(false)
    }
  }, [])

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

  const handleSelectCurrent = () => {
    if (!listing) return
    onSelect(listing.path)
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>{t("chat.workingDir.browserTitle")}</DialogTitle>
          <DialogDescription>
            {t("chat.workingDir.browserDescription")}
          </DialogDescription>
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
          <Input
            value={manualPath}
            onChange={(e) => setManualPath(e.target.value)}
            placeholder={t("chat.workingDir.pathPlaceholder")}
            className="flex-1 font-mono text-xs"
          />
          <Button type="submit" variant="outline" size="sm" disabled={loading}>
            {loading ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <RefreshCw className="h-4 w-4" />
            )}
          </Button>
        </form>

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
