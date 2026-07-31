import type { SkillIconKind } from "./skillTokens"

/**
 * Keep skill mentions visually close to surrounding prose: the icon carries
 * most of the identity, while the label gets a restrained brand-adjacent tone.
 * Composer widgets and rendered messages both consume this helper so they do
 * not drift back into separate chip styles.
 */
export function skillMentionToneClass(kind: SkillIconKind): string {
  switch (kind) {
    case "docx":
      return "text-blue-600 dark:text-blue-400"
    case "pptx":
      return "text-orange-700 dark:text-orange-400"
    case "xlsx":
      return "text-emerald-700 dark:text-emerald-400"
    case "analytics":
      return "text-sky-700 dark:text-sky-400"
    case "browser":
    case "mac":
      return "text-foreground/85"
  }
}
