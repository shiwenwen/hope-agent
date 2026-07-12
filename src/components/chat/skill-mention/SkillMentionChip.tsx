/**
 * Inline rose chip for a `@skill` mention rendered inside message markdown.
 * `MarkdownLink` (common/MarkdownRenderer) dispatches `[@label](#skill:<name>)`
 * links here so the **history bubble shows the same styled chip as the
 * composer** instead of the raw `[@…](#skill:…)` link text. Label + icon are
 * resolved from the catalog by id (current UI language), so it stays localized.
 */

import { useTranslation } from "react-i18next"

import { cn } from "@/lib/utils"
import { SkillMentionIcon } from "./SkillMentionIcon"
import { skillMentionMeta } from "./skillTokens"

export function SkillMentionChip({ name }: { name: string }) {
  const { t } = useTranslation()
  const meta = skillMentionMeta(name)
  if (!meta) return null
  const label = t(meta.labelKey)
  return (
    <span
      data-skill-mention={name}
      data-ha-title-tip={label}
      className={cn(
        "mx-0.5 inline-flex items-center gap-1 rounded-md border px-1.5 align-baseline",
        "text-[0.95em] font-medium leading-snug",
        "border-rose-500/20 bg-rose-500/10 text-rose-600",
        "dark:border-rose-300/20 dark:bg-rose-300/15 dark:text-rose-200",
      )}
    >
      <SkillMentionIcon kind={meta.iconKind} className="h-3.5 w-3.5 shrink-0" />
      {label}
    </span>
  )
}
