import { useMemo } from "react"
import { useTranslation } from "react-i18next"
import { FilePen } from "lucide-react"

import type { FileChangeMetadata, FileChangesMetadata } from "@/types/chat"
import { cn } from "@/lib/utils"
import { UnifiedDiffView } from "@/components/chat/diff-panel/UnifiedDiffView"
import {
  buildUnifiedRows,
  type DiffViewItem,
  type UnifiedRow,
} from "@/components/chat/diff-panel/diffLayout"
import { PANEL_SCROLL_FADE } from "@/components/chat/right-panel/panelFade"
import { FileDeltaCounter } from "@/components/chat/message/FileDeltaCounter"

interface InlineToolDiffPreviewProps {
  payload: FileChangeMetadata | FileChangesMetadata
  className?: string
}

function toChanges(payload: FileChangeMetadata | FileChangesMetadata): FileChangeMetadata[] {
  return payload.kind === "file_change" ? [payload] : payload.changes
}

function actionLabel(action: FileChangeMetadata["action"]): string {
  switch (action) {
    case "create":
      return "create"
    case "delete":
      return "delete"
    default:
      return "edit"
  }
}

function ChangeSection({ change }: { change: FileChangeMetadata }) {
  const { t } = useTranslation()
  const diffItems = useMemo<DiffViewItem<UnifiedRow>[]>(
    () =>
      buildUnifiedRows(change.before ?? "", change.after ?? "").map((row, rowIndex) => ({
        kind: "row",
        row,
        rowIndex,
      })),
    [change.after, change.before],
  )

  return (
    <section className="min-w-0 border-b border-border/50 last:border-b-0">
      <div className="sticky top-0 z-10 flex min-w-0 items-center gap-2 border-b border-border/50 bg-secondary/95 px-2.5 py-1.5 text-[11px] backdrop-blur">
        <FilePen className="h-3 w-3 shrink-0 text-muted-foreground/70" />
        <span className="shrink-0 rounded bg-background/70 px-1.5 py-0.5 font-mono text-[10px] uppercase text-muted-foreground">
          {actionLabel(change.action)}
        </span>
        <span
          className="min-w-0 flex-1 truncate font-mono text-muted-foreground"
          title={change.path}
        >
          {change.path}
        </span>
        <FileDeltaCounter
          linesAdded={change.linesAdded}
          linesRemoved={change.linesRemoved}
          className="text-[11px]"
        />
      </div>
      {change.truncated && (
        <div className="border-b border-border/40 bg-amber-500/10 px-2.5 py-1 text-[11px] text-amber-700 dark:text-amber-300">
          {t("diffPanel.fileTooLarge", "文件过大，仅渲染前 256KB")}
        </div>
      )}
      <UnifiedDiffView
        items={diffItems}
        omittedItemCount={0}
        onToggleFold={() => undefined}
        onRenderAll={() => undefined}
        onCopyLocation={() => undefined}
        onOpenLocation={() => undefined}
      />
    </section>
  )
}

export default function InlineToolDiffPreview({ payload, className }: InlineToolDiffPreviewProps) {
  const { t } = useTranslation()
  const changes = toChanges(payload)

  if (changes.length === 0) {
    return (
      <div
        className={cn(
          "rounded-md border border-border/50 bg-secondary/40 p-2.5 text-[11px] text-muted-foreground",
          className,
        )}
      >
        {t("diffPanel.noDiffData", "无 diff 数据")}
      </div>
    )
  }

  return (
    <div
      className={cn(
        "h-72 overflow-auto rounded-md border border-border/50 bg-secondary/40",
        PANEL_SCROLL_FADE,
        className,
      )}
    >
      {changes.map((change, idx) => (
        <ChangeSection key={`${change.path}-${idx}`} change={change} />
      ))}
    </div>
  )
}
