/**
 * Sidebar section listing projects.
 *
 * Rendered above AgentSection in the sidebar. Each project is a clickable
 * row that opens ProjectOverviewDialog. A "+" button opens ProjectDialog
 * in create mode.
 */

import { useTranslation } from "react-i18next"
import { ChevronDown, ChevronRight, FolderKanban, Plus } from "lucide-react"

import { IconTip } from "@/components/ui/tooltip"
import type { ProjectMeta } from "@/types/project"

interface ProjectSectionProps {
  projects: ProjectMeta[]
  expanded: boolean
  setExpanded: (v: boolean) => void
  onAddProject: () => void
  onOpenProject: (project: ProjectMeta) => void
}

const COLOR_MAP: Record<string, string> = {
  amber: "bg-amber-500/15 text-amber-600 dark:text-amber-400",
  violet: "bg-violet-500/15 text-violet-600 dark:text-violet-400",
  sky: "bg-sky-500/15 text-sky-600 dark:text-sky-400",
  emerald: "bg-emerald-500/15 text-emerald-600 dark:text-emerald-400",
  rose: "bg-rose-500/15 text-rose-600 dark:text-rose-400",
  indigo: "bg-indigo-500/15 text-indigo-600 dark:text-indigo-400",
  slate: "bg-slate-500/15 text-slate-600 dark:text-slate-400",
}

export default function ProjectSection({
  projects,
  expanded,
  setExpanded,
  onAddProject,
  onOpenProject,
}: ProjectSectionProps) {
  const { t } = useTranslation()
  const visibleProjects = projects.filter((p) => !p.archived)

  return (
    <div className="px-3 pt-3">
      <div className="flex items-center gap-1 mb-2">
        <button
          onClick={() => setExpanded(!expanded)}
          className="flex items-center gap-1 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground/80 hover:text-foreground transition-colors"
        >
          {expanded ? (
            <ChevronDown className="h-3 w-3" />
          ) : (
            <ChevronRight className="h-3 w-3" />
          )}
          {t("project.projects")}
          {visibleProjects.length > 0 && (
            <span className="ml-1 text-muted-foreground/60">
              · {visibleProjects.length}
            </span>
          )}
        </button>
        <div className="ml-auto">
          <IconTip label={t("project.newProject")}>
            <button
              onClick={onAddProject}
              className="text-muted-foreground/60 hover:text-foreground transition-colors"
            >
              <Plus className="h-3.5 w-3.5" />
            </button>
          </IconTip>
        </div>
      </div>

      {expanded && (
        <div className="space-y-0.5">
          {visibleProjects.length === 0 ? (
            <button
              onClick={onAddProject}
              className="w-full text-left text-xs text-muted-foreground/70 italic px-2 py-1.5 rounded-md hover:bg-accent/40"
            >
              {t("project.createFirstProject")}
            </button>
          ) : (
            visibleProjects.map((project) => (
              <ProjectRow
                key={project.id}
                project={project}
                onClick={() => onOpenProject(project)}
              />
            ))
          )}
        </div>
      )}
    </div>
  )
}

function ProjectRow({
  project,
  onClick,
}: {
  project: ProjectMeta
  onClick: () => void
}) {
  const colorClass = project.color && COLOR_MAP[project.color]
    ? COLOR_MAP[project.color]
    : "bg-primary/15 text-primary"

  return (
    <button
      onClick={onClick}
      className="w-full flex items-center gap-2 px-2 py-1.5 rounded-md hover:bg-accent/40 transition-colors text-left"
    >
      <div
        className={`w-6 h-6 rounded-md flex items-center justify-center shrink-0 text-xs overflow-hidden ${
          project.logo ? "" : colorClass
        }`}
      >
        {project.logo ? (
          <img
            src={project.logo}
            alt=""
            className="w-full h-full object-cover"
          />
        ) : project.emoji ? (
          <span className="text-sm">{project.emoji}</span>
        ) : (
          <FolderKanban className="h-3.5 w-3.5" />
        )}
      </div>
      <div className="flex-1 min-w-0">
        <div className="text-sm truncate text-foreground/90">{project.name}</div>
      </div>
      {project.sessionCount > 0 && (
        <span className="text-[10px] text-muted-foreground/70 tabular-nums">
          {project.sessionCount}
        </span>
      )}
    </button>
  )
}
