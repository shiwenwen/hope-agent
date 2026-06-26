/** Tailwind classes that tint the project icon chip. Keyed by `Project.color`. */
export const PROJECT_COLOR_MAP: Record<string, string> = {
  amber: "bg-amber-500/15 text-amber-600 dark:text-amber-400",
  violet: "bg-violet-500/15 text-violet-600 dark:text-violet-400",
  sky: "bg-sky-500/15 text-sky-600 dark:text-sky-400",
  emerald: "bg-emerald-500/15 text-emerald-600 dark:text-emerald-400",
  rose: "bg-rose-500/15 text-rose-600 dark:text-rose-400",
  indigo: "bg-indigo-500/15 text-indigo-600 dark:text-indigo-400",
  slate: "bg-slate-500/15 text-slate-600 dark:text-slate-400",
}

/** Text-only project color classes for inline icons. Keyed by `Project.color`. */
export const PROJECT_TEXT_COLOR_MAP: Record<string, string> = {
  amber: "text-amber-600 dark:text-amber-400",
  violet: "text-violet-600 dark:text-violet-400",
  sky: "text-sky-600 dark:text-sky-400",
  emerald: "text-emerald-600 dark:text-emerald-400",
  rose: "text-rose-600 dark:text-rose-400",
  indigo: "text-indigo-600 dark:text-indigo-400",
  slate: "text-slate-600 dark:text-slate-400",
}
