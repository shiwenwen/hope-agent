/**
 * Shared glyph for a `@skill` catalog entry, used by both the composer chip
 * ({@link MentionComposerInput}) and the `@` menu row ({@link FileMentionMenu}).
 * Every entry renders its real, colorful brand mark (inlined at build time by
 * `unplugin-icons`, offline / CSP-safe): office skills reuse the vscode-icons
 * `FileTypeIcon`, browser uses the Chrome logo, mac control a flat colorful
 * MacBook. All carry explicit fills, so they ignore the caller's rose text tint,
 * and all sit in a square viewBox that lines up with the square office icons.
 */

import { FileTypeIcon } from "@/components/icons/FileTypeIcon"
import IconChrome from "~icons/logos/chrome"
import IconMacbook from "~icons/fluent-emoji-flat/laptop"
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
      return <IconChrome className={className} />
    case "mac":
      return <IconMacbook className={className} />
  }
}
