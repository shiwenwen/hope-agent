import { cn } from "@/lib/utils"
import type { FileChangeMetadata } from "@/types/chat"
import { buildSplitRows } from "./diffLayout"

interface SplitDiffViewProps {
  change: FileChangeMetadata
}

/**
 * Two-column diff view with old on the left and new on the right. Adjacent
 * removed/added blocks pair across rows; lone removed/added lines occupy
 * only their column with a blank counterpart.
 */
export function SplitDiffView({ change }: SplitDiffViewProps) {
  const rows = buildSplitRows(change.before ?? "", change.after ?? "")

  return (
    <div className="font-mono text-[11.5px] leading-5">
      {rows.map((row, idx) => {
        const leftBg = row.left
          ? row.left.type === "removed"
            ? "bg-rose-500/10"
            : ""
          : "bg-muted/20"
        const rightBg = row.right
          ? row.right.type === "added"
            ? "bg-emerald-500/10"
            : ""
          : "bg-muted/20"
        return (
          <div key={idx} className="flex items-start">
            <div
              className={cn(
                "flex flex-1 min-w-0 items-start border-r border-border/40 whitespace-pre",
                leftBg,
                row.left?.type === "removed" && "text-rose-700 dark:text-rose-300",
              )}
            >
              <span className="shrink-0 w-10 select-none px-1.5 text-right tabular-nums text-muted-foreground/60">
                {row.left?.lineNumber ?? ""}
              </span>
              <span className="flex-1 whitespace-pre-wrap break-all px-2">
                {row.left ? row.left.text || " " : ""}
              </span>
            </div>
            <div
              className={cn(
                "flex flex-1 min-w-0 items-start whitespace-pre",
                rightBg,
                row.right?.type === "added" && "text-emerald-700 dark:text-emerald-300",
              )}
            >
              <span className="shrink-0 w-10 select-none px-1.5 text-right tabular-nums text-muted-foreground/60">
                {row.right?.lineNumber ?? ""}
              </span>
              <span className="flex-1 whitespace-pre-wrap break-all px-2">
                {row.right ? row.right.text || " " : ""}
              </span>
            </div>
          </div>
        )
      })}
    </div>
  )
}
