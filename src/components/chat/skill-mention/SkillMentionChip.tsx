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
        "mx-0.5 inline-flex max-w-[16rem] items-baseline gap-1 whitespace-nowrap align-baseline",
        "font-normal leading-[inherit]",
        skillMentionToneClass(meta.iconKind),
      )}
    >
      <SkillMentionIcon kind={meta.iconKind} className="h-[1em] w-[1em] shrink-0 self-center" />
      <span className="min-w-0 truncate">{label}</span>
    </span>
  )
}
