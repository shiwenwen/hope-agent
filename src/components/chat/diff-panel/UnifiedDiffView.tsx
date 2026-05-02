import { cn } from "@/lib/utils"
import type { FileChangeMetadata } from "@/types/chat"
import { buildUnifiedRows } from "./diffLayout"

interface UnifiedDiffViewProps {
  change: FileChangeMetadata
}

/**
 * Single-column diff view. Removed lines render on a red row, added on
 * green, context unstyled. Each row carries the corresponding line numbers
 * from the old and new files so the user can map back to source.
 */
export function UnifiedDiffView({ change }: UnifiedDiffViewProps) {
  const rows = buildUnifiedRows(change.before ?? "", change.after ?? "")

  return (
    <div className="font-mono text-[11.5px] leading-5">
      {rows.map((row, idx) => {
        const bg =
          row.type === "added"
            ? "bg-emerald-500/10"
            : row.type === "removed"
              ? "bg-rose-500/10"
              : ""
        const marker = row.type === "added" ? "+" : row.type === "removed" ? "-" : " "
        return (
          <div
            key={idx}
            className={cn(
              "flex items-start whitespace-pre",
              bg,
              row.type === "added" && "text-emerald-700 dark:text-emerald-300",
              row.type === "removed" && "text-rose-700 dark:text-rose-300",
            )}
          >
            <span className="shrink-0 w-10 select-none px-1.5 text-right tabular-nums text-muted-foreground/60">
              {row.oldLineNumber ?? ""}
            </span>
            <span className="shrink-0 w-10 select-none px-1.5 text-right tabular-nums text-muted-foreground/60">
              {row.newLineNumber ?? ""}
            </span>
            <span className="shrink-0 w-4 select-none text-center text-muted-foreground/60">
              {marker}
            </span>
            <span className="flex-1 whitespace-pre-wrap break-all px-2">{row.text || " "}</span>
          </div>
        )
      })}
    </div>
  )
}
