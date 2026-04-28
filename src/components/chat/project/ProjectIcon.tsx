import { FolderKanban } from "lucide-react"
import { cn } from "@/lib/utils"
import type { Project } from "@/types/project"
import { PROJECT_COLOR_MAP } from "./colors"

/**
 * Visual identity badge for a project: prefer logo (data URL), fall back to
 * emoji, then to the FolderKanban icon. Used in the sidebar tree row, the
 * settings sheet header, and the title-bar chip.
 *
 * `withColorChip` wraps the icon in a tinted square (sidebar style); without
 * it the icon renders inline with no background (used in title bar / sheet
 * header where the surrounding layout already provides framing).
 */

type IconSize = "xs" | "sm" | "md" | "lg"

const SIZE_PRESETS: Record<IconSize, { box: string; emoji: string; lucide: string }> = {
  xs: { box: "w-3.5 h-3.5", emoji: "text-[11px]", lucide: "h-3 w-3" },
  sm: { box: "w-6 h-6", emoji: "text-sm", lucide: "h-3.5 w-3.5" },
  md: { box: "w-8 h-8", emoji: "text-xl", lucide: "h-4 w-4" },
  lg: { box: "w-10 h-10", emoji: "text-3xl", lucide: "h-5 w-5" },
}

interface ProjectIconProps {
  project: Pick<Project, "logo" | "emoji" | "color">
  size?: IconSize
  /** Wrap the icon in a tinted background square keyed off `project.color`. */
  withColorChip?: boolean
  className?: string
}

export default function ProjectIcon({
  project,
  size = "sm",
  withColorChip = false,
  className,
}: ProjectIconProps) {
  const preset = SIZE_PRESETS[size]
  const colorClass =
    withColorChip && !project.logo
      ? (project.color && PROJECT_COLOR_MAP[project.color]) || "bg-primary/15 text-primary"
      : ""
  const wrapperClass = cn(
    preset.box,
    "shrink-0 overflow-hidden flex items-center justify-center",
    withColorChip && "rounded-md",
    colorClass,
    className,
  )

  if (project.logo) {
    return <img src={project.logo} alt="" className={cn(wrapperClass, "object-cover")} />
  }
  if (project.emoji) {
    return (
      <span className={wrapperClass}>
        <span className={cn("leading-none", preset.emoji)}>{project.emoji}</span>
      </span>
    )
  }
  return (
    <span className={wrapperClass}>
      <FolderKanban className={preset.lucide} />
    </span>
  )
}
