import { useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Loader2 } from "lucide-react"
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
import { Button } from "@/components/ui/button"
import { formatBytes, formatDurationCompact } from "@/lib/format"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog"
import { Progress } from "@/components/ui/progress"
import type { ProgressFrame } from "@/types/local-model-jobs"

export type { ProgressFrame }

export function InstallProgressDialog({
  open,
  onOpenChange,
  title,
  subtitle,
  frame,
  logs,
  done,
  error,
  cancellable,
  onBackground,
  onCancelTask,
}: {
  open: boolean
  onOpenChange?: (open: boolean) => void
  title: string
  subtitle?: string
  frame: ProgressFrame | null
  logs: string[]
  done: boolean
  error: string | null
  cancellable?: boolean
  onBackground?: () => void
  onCancelTask?: () => void
}) {
  const { t } = useTranslation()
  const tailRef = useRef<HTMLDivElement | null>(null)
  const previousBytesRef = useRef<{ bytes: number; timestamp: number } | null>(null)
  const [confirmCloseOpen, setConfirmCloseOpen] = useState(false)
  const [transfer, setTransfer] = useState<{ speedBps?: number; etaSeconds?: number }>({})

  useEffect(() => {
    tailRef.current?.scrollTo({ top: tailRef.current.scrollHeight })
  }, [logs])

  const indeterminate = !error && !done && (frame?.percent == null || Number.isNaN(frame.percent))
  const running = !done && !error
  const shouldConfirmClose = running && Boolean(onBackground || onCancelTask)
  const canClose = Boolean(done || error || cancellable || shouldConfirmClose)

  useEffect(() => {
    const completed = frame?.bytesCompleted ?? null
    const total = frame?.bytesTotal ?? null
    if (!running || completed == null || completed < 0) {
      previousBytesRef.current = null
      const timeout = window.setTimeout(() => setTransfer({}), 0)
      return () => window.clearTimeout(timeout)
    }

    const now = Date.now()
    const previous = previousBytesRef.current
    previousBytesRef.current = { bytes: completed, timestamp: now }

    if (!previous || completed <= previous.bytes || now <= previous.timestamp) return

    const elapsedSeconds = (now - previous.timestamp) / 1000
    if (elapsedSeconds <= 0) return

    const speedBps = (completed - previous.bytes) / elapsedSeconds
    if (!Number.isFinite(speedBps) || speedBps <= 0) return

    const etaSeconds =
      total != null && total > completed ? (total - completed) / speedBps : undefined
    const timeout = window.setTimeout(() => setTransfer({ speedBps, etaSeconds }), 0)
    return () => window.clearTimeout(timeout)
  }, [frame?.bytesCompleted, frame?.bytesTotal, running])

  const transferSummary = (() => {
    const completed = frame?.bytesCompleted ?? null
    const total = frame?.bytesTotal ?? null
    if (completed == null && total == null) return null
    const parts: string[] = []
    if (completed != null && total != null) {
      parts.push(`${formatBytes(completed, { maxUnit: "GB" })} / ${formatBytes(total, { maxUnit: "GB" })}`)
    } else if (completed != null) {
      parts.push(formatBytes(completed, { maxUnit: "GB" }))
    }
    if (transfer.speedBps) {
      parts.push(`${formatBytes(transfer.speedBps, { maxUnit: "GB" })}/s`)
    }
    if (transfer.etaSeconds != null && Number.isFinite(transfer.etaSeconds)) {
      parts.push(`≈ ${formatDurationCompact(transfer.etaSeconds)}`)
    }
    return parts.join(" · ")
  })()

  const requestClose = () => {
    if (shouldConfirmClose) {
      setConfirmCloseOpen(true)
      return
    }
    if (canClose) onOpenChange?.(false)
  }

  const background = () => {
    setConfirmCloseOpen(false)
    onBackground?.()
    onOpenChange?.(false)
  }

  const cancelTask = () => {
    setConfirmCloseOpen(false)
    onCancelTask?.()
    onOpenChange?.(false)
  }

  return (
    <>
      <Dialog
        open={open}
        onOpenChange={(nextOpen) => {
          if (nextOpen) {
            onOpenChange?.(true)
            return
          }
          requestClose()
        }}
      >
        <DialogContent
          className="sm:max-w-lg"
          onEscapeKeyDown={(e) => {
            if (!canClose) e.preventDefault()
          }}
          onInteractOutside={(e) => {
            if (!canClose) e.preventDefault()
          }}
        >
          <DialogHeader>
            <DialogTitle>{title}</DialogTitle>
            {subtitle && <DialogDescription>{subtitle}</DialogDescription>}
          </DialogHeader>

          <div className="space-y-3 pt-2">
            <Progress value={frame?.percent ?? null} indeterminate={indeterminate} />
            <div className="flex items-center justify-between text-xs text-muted-foreground">
              <span className="flex items-center gap-2 truncate">
                {!done && !error && <Loader2 className="h-3 w-3 animate-spin shrink-0" />}
                <span className="truncate">{frame?.message ?? frame?.phase ?? "…"}</span>
              </span>
              {frame?.percent != null && !error && (
                <span className="tabular-nums">{Math.round(frame.percent)}%</span>
              )}
            </div>
            {transferSummary && (
              <div className="text-xs tabular-nums text-muted-foreground">
                {transferSummary}
              </div>
            )}
            {logs.length > 0 && (
              <div
                ref={tailRef}
                className="max-h-40 overflow-y-auto rounded-md border border-border/60 bg-muted/40 p-2 font-mono text-[11px] leading-tight text-muted-foreground"
              >
                {logs.map((line, i) => (
                  <div key={i} className="whitespace-pre-wrap break-all">
                    {line}
                  </div>
                ))}
              </div>
            )}
            {error && <p className="text-xs text-destructive whitespace-pre-wrap">{error}</p>}
            {!cancellable && !done && !error && !onBackground && !onCancelTask && (
              <p className="text-[11px] text-muted-foreground/70">
                {t("settings.localLlm.install.cannotCancel")}
              </p>
            )}
            {running && (onBackground || onCancelTask) && (
              <div className="flex justify-end gap-2 pt-1">
                {onCancelTask && (
                  <Button type="button" variant="destructive" size="sm" onClick={cancelTask}>
                    {t("localModelJobs.actions.cancelInstall")}
                  </Button>
                )}
                {onBackground && (
                  <Button type="button" variant="secondary" size="sm" onClick={background}>
                    {t("localModelJobs.actions.backgroundInstall")}
                  </Button>
                )}
              </div>
            )}
          </div>
        </DialogContent>
      </Dialog>

      <AlertDialog open={confirmCloseOpen} onOpenChange={setConfirmCloseOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("localModelJobs.close.title")}</AlertDialogTitle>
            <AlertDialogDescription>{t("localModelJobs.close.description")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("localModelJobs.actions.keepWatching")}</AlertDialogCancel>
            {onCancelTask && (
              <Button type="button" variant="destructive" onClick={cancelTask}>
                {t("localModelJobs.actions.cancelInstall")}
              </Button>
            )}
            {onBackground && (
              <AlertDialogAction onClick={background}>
                {t("localModelJobs.actions.backgroundInstall")}
              </AlertDialogAction>
            )}
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}
