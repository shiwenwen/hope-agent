import { FileText, Folder } from "lucide-react"

import { cn } from "@/lib/utils"
import { basename } from "@/lib/path"

export const FILE_MENTION_INLINE_CLASS =
  "mx-0.5 inline-flex max-w-[16rem] items-baseline gap-1 whitespace-nowrap align-baseline font-normal leading-[inherit] text-[#339CFF]"
export const FILE_MENTION_ICON_CLASS = "h-[1em] w-[1em] shrink-0 self-center"

function fileMentionLabel(targetId: string, displayLabel?: string): string {
  if (displayLabel) return displayLabel
  const normalized = targetId.replace(/[\\/]+$/, "")
  return basename(normalized || targetId)
}

export function FileMentionChip({
  targetId,
  displayLabel,
}: {
  targetId: string
  displayLabel?: string
}) {
  const label = fileMentionLabel(targetId, displayLabel)
  const Icon = /[\\/]$/.test(targetId) ? Folder : FileText

  return (
    <span
      data-file-mention={targetId}
      data-ha-title-tip={targetId}
      className={cn(FILE_MENTION_INLINE_CLASS)}
    >
      <Icon className={FILE_MENTION_ICON_CLASS} />
      <span className="min-w-0 truncate">{label}</span>
    </span>
  )
}
