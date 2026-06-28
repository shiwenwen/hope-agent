import { useTranslation } from "react-i18next"
import { ChevronDown } from "lucide-react"
import { cn } from "@/lib/utils"
import type { DiffViewItem, InlineToken, UnifiedRow } from "./diffLayout"

type DiffLineSide = "old" | "new"

interface UnifiedDiffViewProps {
  items: DiffViewItem<UnifiedRow>[]
  omittedItemCount: number
  onToggleFold: (id: string) => void
  onRenderAll: () => void
  onCopyLocation: (line: number, side: DiffLineSide) => void
  onOpenLocation: (line: number, side: DiffLineSide) => void
}

/**
 * Single-column diff view. Removed lines render on a red row, added on
 * green, context unstyled. Each row carries the corresponding line numbers
 * from the old and new files so the user can map back to source.
 */
export function UnifiedDiffView({
  items,
  omittedItemCount,
  onToggleFold,
  onRenderAll,
  onCopyLocation,
  onOpenLocation,
}: UnifiedDiffViewProps) {
  const { t } = useTranslation()

  return (
    <div className="font-mono text-[11.5px] leading-5">
      {items.map((item) => {
        if (item.kind === "fold") {
          return (
            <FoldRow
              key={`fold-${item.id}`}
              hiddenCount={item.hiddenCount}
              onClick={() => onToggleFold(item.id)}
            />
          )
        }

        const row = item.row
        const changed = row.type === "added" || row.type === "removed"
        const bg =
          row.type === "added"
            ? "bg-emerald-500/10"
            : row.type === "removed"
              ? "bg-rose-500/10"
              : ""
        const marker =
          row.type === "added" ? "+" : row.type === "removed" ? "-" : " "
        return (
          <div
            key={`row-${item.rowIndex}`}
            data-diff-row={changed ? "true" : undefined}
            data-diff-hunk-index={row.hunkIndex ?? undefined}
            data-diff-hunk-start={row.isHunkStart ? "true" : undefined}
            data-first-diff-row={row.hunkIndex === 0 && row.isHunkStart ? "true" : undefined}
            className={cn(
              "group/diff-row flex items-start whitespace-pre",
              bg,
              row.type === "added" && "text-emerald-700 dark:text-emerald-300",
              row.type === "removed" && "text-rose-700 dark:text-rose-300",
            )}
          >
            <LineNumber
              line={row.oldLineNumber}
              side="old"
              onCopyLocation={onCopyLocation}
              onOpenLocation={onOpenLocation}
            />
            <LineNumber
              line={row.newLineNumber}
              side="new"
              onCopyLocation={onCopyLocation}
              onOpenLocation={onOpenLocation}
            />
            <span className="shrink-0 w-4 select-none text-center text-muted-foreground/60">
              {marker}
            </span>
            <span className="flex-1 whitespace-pre-wrap break-all px-2">
              <InlineText text={row.text} tokens={row.inlineTokens} tone={row.type} />
            </span>
          </div>
        )
      })}
      {omittedItemCount > 0 && (
        <button
          type="button"
          onClick={onRenderAll}
          className="flex w-full items-center justify-center gap-1 border-t border-border/50 px-3 py-2 text-xs text-muted-foreground transition-colors hover:bg-secondary/50 hover:text-foreground"
        >
          <ChevronDown className="h-3.5 w-3.5" />
          {t("diffPanel.renderMore", "显示剩余 {{count}} 行", { count: omittedItemCount })}
        </button>
      )}
    </div>
  )
}

function FoldRow({ hiddenCount, onClick }: { hiddenCount: number; onClick: () => void }) {
  const { t } = useTranslation()
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex w-full items-center gap-2 border-y border-border/40 bg-muted/25 px-3 py-1 text-left text-[11px] text-muted-foreground transition-colors hover:bg-secondary/55 hover:text-foreground"
    >
      <ChevronDown className="h-3.5 w-3.5 shrink-0" />
      <span className="h-px flex-1 bg-border/60" />
      <span className="shrink-0">
        {t("diffPanel.expandHiddenLines", "展开 {{count}} 行未变更内容", { count: hiddenCount })}
      </span>
      <span className="h-px flex-1 bg-border/60" />
    </button>
  )
}

function LineNumber({
  line,
  side,
  onCopyLocation,
  onOpenLocation,
}: {
  line?: number
  side: DiffLineSide
  onCopyLocation: (line: number, side: DiffLineSide) => void
  onOpenLocation: (line: number, side: DiffLineSide) => void
}) {
  if (!line) {
    return (
      <span className="shrink-0 w-10 select-none px-1.5 text-right tabular-nums text-muted-foreground/60" />
    )
  }

  return (
    <button
      type="button"
      className="shrink-0 w-10 select-none px-1.5 text-right tabular-nums text-muted-foreground/60 transition-colors hover:bg-secondary/70 hover:text-foreground"
      onClick={() => onOpenLocation(line, side)}
      onContextMenu={(e) => {
        e.preventDefault()
        onCopyLocation(line, side)
      }}
    >
      {line}
    </button>
  )
}

function InlineText({
  text,
  tokens,
  tone,
}: {
  text: string
  tokens?: InlineToken[]
  tone: UnifiedRow["type"]
}) {
  if (!tokens?.some((token) => token.changed)) return <>{text || " "}</>
  const highlight =
    tone === "added"
      ? "bg-emerald-500/20 text-emerald-800 dark:text-emerald-200"
      : "bg-rose-500/20 text-rose-800 dark:text-rose-200"
  return (
    <>
      {tokens.map((token, idx) => (
        <span key={idx} className={token.changed ? highlight : undefined}>
          {token.text || " "}
        </span>
      ))}
    </>
  )
}
