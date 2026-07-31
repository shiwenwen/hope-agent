/**
 * Lightweight inline treatment for a `@skill` mention in message markdown.
 * `MarkdownLink` (common/MarkdownRenderer) dispatches `[@label](#skill:<name>)`
 * links here so the **history bubble shows the same styled mention as the
 * composer** instead of the raw `[@…](#skill:…)` link text. Label + icon are
 * resolved from the catalog by id (current UI language), so it stays localized.
 */

import { useTranslation } from "react-i18next"

import { cn } from "@/lib/utils"
import { SkillMentionIcon } from "./SkillMentionIcon"
import { skillMentionToneClass } from "./skillMentionStyles"
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
        "mx-0.5 inline-flex max-w-[16rem] items-center gap-1 whitespace-nowrap align-baseline",
        "text-[0.95em] font-normal leading-snug",
        skillMentionToneClass(meta.iconKind),
      )}
    >
      <SkillMentionIcon kind={meta.iconKind} className="h-3.5 w-3.5 shrink-0" />
      {label}
    </span>
  )
}
