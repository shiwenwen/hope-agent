import type { LucideIcon } from "lucide-react"

export type WorkbenchPanelId =
  | "workspace"
  | "pull-request"
  | "diff"
  | "plan"
  | "files"
  | "browser"
  | "mac-control"
  | "canvas"
  | "team"
  | "background-jobs"
  | "subagent"
  | "preview"

export interface WorkbenchBadge {
  count: number
  labelKey: string
  tone: "attention" | "running" | "neutral"
}

export interface WorkbenchTabItem {
  /** Unique tab identity. File previews use one id per open file. */
  id: string
  /** Owning panel surface. Several tabs may share the preview surface. */
  panelId: WorkbenchPanelId
  labelKey?: string
  label?: string
  icon: LucideIcon
  /** File-preview tabs show the colorful format icon instead of `icon`. */
  fileIcon?: { name: string; mime?: string | null }
  open: boolean
  badge?: WorkbenchBadge
  /** Panels with an in-app floating mode expose that action on the tab. */
  windowMode?: "docked" | "floating" | "detached"
}

export type WorkbenchWidthMode = "auto" | "manual"
export type WorkbenchLayoutMode = "docked" | "stage"
