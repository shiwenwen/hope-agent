import { useTranslation } from "react-i18next"

interface PanelSessionStatsProps {
  steps: number
  failed: number
  totalMs: number
  currentTarget: string | null
}

function formatTotal(ms: number): string {
  const secs = Math.round(ms / 1000)
  if (secs < 60) return `${secs}s`
  return `${Math.floor(secs / 60)}m${secs % 60 ? ` ${secs % 60}s` : ""}`
}

/** Compact three-cell stats strip for the docked control panels. */
export function PanelSessionStats({
  steps,
  failed,
  totalMs,
  currentTarget,
}: PanelSessionStatsProps) {
  const { t } = useTranslation()
  return (
    <div className="grid grid-cols-3 gap-1.5 border-t border-border/60 px-2 py-1.5">
      <div className="rounded-lg bg-muted/40 px-2 py-1.5">
        <div className="text-[10px] text-muted-foreground">{t("chat.controlPanel.stats.steps")}</div>
        <div className="text-xs font-medium tabular-nums">
          {steps}
          {failed > 0 && (
            <span className="ml-1 text-[10px] text-destructive">
              {t("chat.controlPanel.stats.failed", { n: failed })}
            </span>
          )}
        </div>
      </div>
      <div className="rounded-lg bg-muted/40 px-2 py-1.5">
        <div className="text-[10px] text-muted-foreground">
          {t("chat.controlPanel.stats.duration")}
        </div>
        <div className="text-xs font-medium tabular-nums">{formatTotal(totalMs)}</div>
      </div>
      <div className="min-w-0 rounded-lg bg-muted/40 px-2 py-1.5">
        <div className="text-[10px] text-muted-foreground">
          {t("chat.controlPanel.stats.target")}
        </div>
        <div className="truncate text-xs font-medium">{currentTarget || "—"}</div>
      </div>
    </div>
  )
}
