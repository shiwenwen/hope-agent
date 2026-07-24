import { cn } from "@/lib/utils"

/** Shared flat state surface for single- and multi-select pills. */
export function selectionPillStateClass(selected: boolean, interactive = true): string {
  if (selected) {
    return cn(
      "bg-primary text-primary-foreground",
      interactive && "hover:bg-primary/90",
    )
  }

  return cn(
    "bg-secondary text-secondary-foreground",
    interactive && "hover:bg-foreground/15 hover:text-foreground",
  )
}
