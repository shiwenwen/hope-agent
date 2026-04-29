import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { GitCompare, X, Columns2, Rows3 } from "lucide-react"
import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import type { FileChangeMetadata } from "@/types/chat"
import { UnifiedDiffView } from "./UnifiedDiffView"
import { SplitDiffView } from "./SplitDiffView"

const LAYOUT_STORAGE_KEY = "ha:diff-panel:layout"
type DiffLayout = "unified" | "split"

function readStoredLayout(): DiffLayout {
  if (typeof window === "undefined") return "unified"
  const value = window.localStorage.getItem(LAYOUT_STORAGE_KEY)
  return value === "split" ? "split" : "unified"
}

function persistLayout(value: DiffLayout) {
  try {
    window.localStorage.setItem(LAYOUT_STORAGE_KEY, value)
  } catch {
    // Storage access failures are non-fatal.
  }
}

interface DiffPanelProps {
  changes: FileChangeMetadata[]
  activeIndex: number
  onActiveIndexChange: (index: number) => void
  onClose: () => void
  embedded?: boolean
}

/**
 * Right-side diff panel mirroring the PlanPanel embedded mode. Renders one or
 * more file changes coming from a single tool call (write / edit /
 * apply_patch). Selecting a tab switches the rendered file; the layout
 * toggle remembers the user's choice in localStorage.
 */
export function DiffPanel({
  changes,
  activeIndex,
  onActiveIndexChange,
  onClose,
  embedded = false,
}: DiffPanelProps) {
  const { t } = useTranslation()
  const [layout, setLayout] = useState<DiffLayout>(() => readStoredLayout())

  useEffect(() => {
    persistLayout(layout)
  }, [layout])

  const safeIndex = Math.min(Math.max(0, activeIndex), Math.max(0, changes.length - 1))
  const change = changes[safeIndex]

  const wrapperClasses = cn(
    "flex h-full min-h-0 w-full flex-col rounded-lg border border-border bg-card shadow-sm",
    embedded ? "" : "max-w-4xl",
  )

  return (
    <div className={wrapperClasses}>
      <div className="flex items-center gap-2 border-b border-border px-3 py-2">
        <GitCompare className="h-4 w-4 shrink-0 text-muted-foreground" />
        <span className="text-sm font-medium truncate">
          {t("diffPanel.title", "文件改动")}
        </span>
        <span className="ml-auto inline-flex items-center gap-1 rounded-md border border-border/60 p-0.5">
          <IconTip label={t("diffPanel.layoutUnified", "Unified")}>
            <button
              type="button"
              className={cn(
                "flex h-6 w-7 items-center justify-center rounded text-muted-foreground transition-colors",
                layout === "unified"
                  ? "bg-secondary text-foreground"
                  : "hover:bg-secondary/60",
              )}
              onClick={() => setLayout("unified")}
              aria-pressed={layout === "unified"}
            >
              <Rows3 className="h-3.5 w-3.5" />
            </button>
          </IconTip>
          <IconTip label={t("diffPanel.layoutSplit", "Split")}>
            <button
              type="button"
              className={cn(
                "flex h-6 w-7 items-center justify-center rounded text-muted-foreground transition-colors",
                layout === "split"
                  ? "bg-secondary text-foreground"
                  : "hover:bg-secondary/60",
              )}
              onClick={() => setLayout("split")}
              aria-pressed={layout === "split"}
            >
              <Columns2 className="h-3.5 w-3.5" />
            </button>
          </IconTip>
        </span>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="h-7 w-7 shrink-0"
          onClick={onClose}
          aria-label={t("common.close", "关闭")}
        >
          <X className="h-4 w-4" />
        </Button>
      </div>

      {changes.length > 1 && (
        <div className="flex shrink-0 items-center gap-1 overflow-x-auto border-b border-border bg-muted/30 px-2 py-1.5">
          {changes.map((c, idx) => (
            <button
              key={`${c.path}-${idx}`}
              type="button"
              className={cn(
                "shrink-0 max-w-[260px] truncate rounded-md px-2 py-1 text-xs transition-colors",
                idx === safeIndex
                  ? "bg-secondary text-foreground"
                  : "text-muted-foreground hover:bg-secondary/60",
              )}
              onClick={() => onActiveIndexChange(idx)}
              title={c.path}
            >
              <span className="font-mono">{shortenPath(c.path)}</span>
              <span className="ml-2 tabular-nums text-emerald-600">
                +{c.linesAdded}
              </span>
              <span className="ml-1 tabular-nums text-rose-600">
                -{c.linesRemoved}
              </span>
            </button>
          ))}
        </div>
      )}

      {change ? (
        <>
          <div className="shrink-0 border-b border-border/60 px-3 py-1.5 text-[11px] text-muted-foreground">
            <div className="flex items-center gap-2 truncate">
              <span className="font-mono truncate">{change.path}</span>
              <span className="ml-auto tabular-nums text-emerald-600">
                +{change.linesAdded}
              </span>
              <span className="tabular-nums text-rose-600">
                -{change.linesRemoved}
              </span>
            </div>
            {change.truncated && (
              <div className="mt-0.5 text-amber-600">
                {t("diffPanel.fileTooLarge", "文件过大，仅渲染前 256KB")}
              </div>
            )}
          </div>
          <div className="flex-1 overflow-auto">
            {layout === "unified" ? (
              <UnifiedDiffView change={change} />
            ) : (
              <SplitDiffView change={change} />
            )}
          </div>
        </>
      ) : (
        <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
          {t("diffPanel.noDiffData", "无 diff 数据")}
        </div>
      )}
    </div>
  )
}

/** Compact a long file path to its tail segments to keep tab labels readable. */
function shortenPath(path: string): string {
  const segments = path.replace(/\\/g, "/").split("/")
  if (segments.length <= 2) return path
  return `…/${segments.slice(-2).join("/")}`
}
