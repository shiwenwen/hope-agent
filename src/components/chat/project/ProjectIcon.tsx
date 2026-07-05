import { cn } from "@/lib/utils"
import type { Project } from "@/types/project"

/**
 * Visual identity badge for a project. Projects without a custom logo render
 * no identity badge.
 */

type IconSize = "xs" | "sm" | "md" | "lg"

const SIZE_PRESETS: Record<
  IconSize,
  { box: string; radius: string }
> = {
  xs: { box: "w-3.5 h-3.5", radius: "rounded-sm" },
  sm: { box: "w-6 h-6", radius: "rounded-md" },
  md: { box: "w-8 h-8", radius: "rounded-md" },
  lg: { box: "w-10 h-10", radius: "rounded-lg" },
}

interface ProjectIconProps {
  project: Pick<Project, "logo">
  size?: IconSize
  className?: string
}

export default function ProjectIcon({
  project,
  size = "sm",
  className,
}: ProjectIconProps) {
  if (!project.logo) return null

  const preset = SIZE_PRESETS[size]
  const wrapperClass = cn(
    preset.box,
    "shrink-0 overflow-hidden flex items-center justify-center",
    preset.radius,
    className,
  )

  return <img src={project.logo} alt="" className={cn(wrapperClass, "object-cover")} />
}
