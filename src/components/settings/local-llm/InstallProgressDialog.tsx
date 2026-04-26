import { useEffect, useRef } from "react"
import { useTranslation } from "react-i18next"
import { Loader2 } from "lucide-react"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog"
import { Progress } from "@/components/ui/progress"

export type ProgressFrame = {
  phase: string
  message?: string
  percent?: number | null
  bytesCompleted?: number | null
  bytesTotal?: number | null
}

export function InstallProgressDialog({
  open,
  title,
  subtitle,
  frame,
  logs,
  done,
  error,
  cancellable,
}: {
  open: boolean
  title: string
  subtitle?: string
  frame: ProgressFrame | null
  logs: string[]
  done: boolean
  error: string | null
  cancellable?: boolean
}) {
  const { t } = useTranslation()
  const tailRef = useRef<HTMLDivElement | null>(null)

  useEffect(() => {
    tailRef.current?.scrollTo({ top: tailRef.current.scrollHeight })
  }, [logs])

  const indeterminate =
    !error && !done && (frame?.percent == null || Number.isNaN(frame.percent))

  return (
    <Dialog open={open}>
      <DialogContent className="sm:max-w-lg" onInteractOutside={(e) => e.preventDefault()}>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          {subtitle && <DialogDescription>{subtitle}</DialogDescription>}
        </DialogHeader>

        <div className="space-y-3 pt-2">
          <Progress
            value={frame?.percent ?? null}
            indeterminate={indeterminate}
          />
          <div className="flex items-center justify-between text-xs text-muted-foreground">
            <span className="flex items-center gap-2 truncate">
              {!done && !error && <Loader2 className="h-3 w-3 animate-spin shrink-0" />}
              <span className="truncate">{frame?.message ?? frame?.phase ?? "…"}</span>
            </span>
            {frame?.percent != null && !error && (
              <span className="tabular-nums">{Math.round(frame.percent)}%</span>
            )}
          </div>
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
          {error && (
            <p className="text-xs text-destructive whitespace-pre-wrap">{error}</p>
          )}
          {!cancellable && !done && !error && (
            <p className="text-[11px] text-muted-foreground/70">
              {t("settings.localLlm.install.cannotCancel")}
            </p>
          )}
        </div>
      </DialogContent>
    </Dialog>
  )
}
