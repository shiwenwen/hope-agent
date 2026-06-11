/**
 * Shared glyph for a `@skill` catalog entry, used by both the composer chip
 * ({@link MentionComposerInput}) and the `@` menu row ({@link FileMentionMenu}).
 * Office skills reuse the colorful `FileTypeIcon` (docx/pptx/xlsx); browser /
 * mac control use a lucide line icon tinted by the caller's `className`.
 */

import { AppWindowMac, Globe } from "lucide-react"

import { FileTypeIcon } from "@/components/icons/FileTypeIcon"
import type { SkillIconKind } from "./skillTokens"

export function SkillMentionIcon({
  kind,
  className,
}: {
  kind: SkillIconKind
  className?: string
}) {
  switch (kind) {
    case "docx":
      return <FileTypeIcon name="a.docx" className={className} />
    case "pptx":
      return <FileTypeIcon name="a.pptx" className={className} />
    case "xlsx":
      return <FileTypeIcon name="a.xlsx" className={className} />
    case "browser":
      return <Globe className={className} />
    case "mac":
      return <AppWindowMac className={className} />
  }
}
