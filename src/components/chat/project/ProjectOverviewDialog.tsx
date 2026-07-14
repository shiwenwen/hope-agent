/**
 * Project settings sheet (formerly `ProjectOverviewDialog`).
 *
 * Slides in from the right as a non-modal-feeling drawer. Tabs:
 * Overview | Files | Instructions | Auto Memory. The old "Sessions" tab is gone — the
 * sidebar now renders project sessions inline as a nested tree node, so
 * having the same list inside this sheet is redundant.
 *
 * The component is exported under its original name so existing imports in
 * `ChatScreen.tsx` keep working without churn; rename to
 * `ProjectSettingsSheet` is left as a follow-up.
 */

import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Pencil, Trash2, Archive, ArchiveRestore } from "lucide-react"

import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import type { Project, ProjectMeta } from "@/types/project"

import { FileBrowserView } from "./file-browser/FileBrowserView"
import ProjectIcon from "./ProjectIcon"
import { ProjectMemorySection } from "./ProjectMemorySection"
import ProjectInstructionsEditor from "./ProjectInstructionsEditor"

interface ProjectOverviewDialogProps {
  open: boolean
  project: ProjectMeta | null
  onOpenChange: (open: boolean) => void
  onEdit: (project: Project) => void
  onDelete: (project: Project) => void
  onArchive: (project: Project, archived: boolean) => void
  onNewSessionInProject: (projectId: string, defaultAgentId?: string | null) => void
  /**
   * Kept in the API for compatibility but no longer used — the Sessions tab
   * was removed because the sidebar now lists project sessions inline.
   */
  onOpenSession?: (sessionId: string) => void
}

export default function ProjectOverviewDialog({
  open,
  project,
  onOpenChange,
  onEdit,
  onDelete,
  onArchive,
  onNewSessionInProject,
}: ProjectOverviewDialogProps) {
  const { t } = useTranslation()
  const [tab, setTab] = useState("overview")

  useEffect(() => {
    if (!open || !project) return
    setTab("overview")
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, project?.id])

  if (!project) return null

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent
        side="right"
        className="w-full sm:max-w-[860px] p-0 flex flex-col"
        // Wider than the default 384px — Project files / instructions need room.
      >
        <SheetHeader className="px-5 pt-5 pb-3 border-b border-border">
          <div className="flex items-start gap-3">
            <ProjectIcon project={project} size="lg" />
            <div className="flex-1 min-w-0 pt-0.5">
              <SheetTitle className="truncate">{project.name}</SheetTitle>
              {project.description && (
                <SheetDescription className="line-clamp-2">{project.description}</SheetDescription>
              )}
            </div>
            <div className="flex items-center gap-0.5 mr-7">
              <IconTip label={t("common.edit")}>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => onEdit(project)}
                  className="h-8 w-8 p-0"
                >
                  <Pencil className="h-3.5 w-3.5" />
                </Button>
              </IconTip>
              <IconTip
                label={
                  project.archived ? t("project.unarchiveProject") : t("project.archiveProject")
                }
              >
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => onArchive(project, !project.archived)}
                  className="h-8 w-8 p-0 text-muted-foreground"
                >
                  {project.archived ? (
                    <ArchiveRestore className="h-3.5 w-3.5" />
                  ) : (
                    <Archive className="h-3.5 w-3.5" />
                  )}
                </Button>
              </IconTip>
              <IconTip label={t("common.delete")}>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => onDelete(project)}
                  className="h-8 w-8 p-0 text-muted-foreground hover:text-destructive"
                >
                  <Trash2 className="h-3.5 w-3.5" />
                </Button>
              </IconTip>
            </div>
          </div>
        </SheetHeader>

        <Tabs value={tab} onValueChange={setTab} className="flex-1 flex flex-col overflow-hidden">
          <TabsList className="shrink-0 mx-5 mt-3 self-start">
            <TabsTrigger value="overview">{t("project.tabOverview")}</TabsTrigger>
            <TabsTrigger value="files">{t("project.tabFiles")}</TabsTrigger>
            <TabsTrigger value="instructions">{t("project.tabInstructions")}</TabsTrigger>
            <TabsTrigger value="auto-memory">{t("project.tabAutoMemory")}</TabsTrigger>
          </TabsList>

          {/* Overview */}
          <TabsContent value="overview" className="flex-1 overflow-y-auto px-5 py-3 space-y-4">
            <div className="grid grid-cols-2 gap-3">
              <StatCard label={t("project.overview.totalSessions")} value={project.sessionCount} />
              <StatCard label={t("project.overview.totalMemories")} value={project.memoryCount} />
            </div>

            {!project.archived && (
              <Button
                onClick={() => {
                  onNewSessionInProject(project.id, project.defaultAgentId)
                  onOpenChange(false)
                }}
                className="w-full"
              >
                {t("project.newChatInProject")}
              </Button>
            )}
          </TabsContent>

          {/* Files */}
          <TabsContent value="files" className="flex-1 overflow-hidden p-0">
            <FileBrowserView
              scope="project"
              scopeId={project.id}
              rootPath={project.workingDir ?? project.id}
              editable
              layout="split"
              className="h-full"
            />
          </TabsContent>

          {/* Instructions */}
          <TabsContent
            value="instructions"
            forceMount
            className="min-h-0 flex-1 overflow-hidden px-5 py-3 data-[state=inactive]:hidden"
          >
            <ProjectInstructionsEditor projectId={project.id} />
          </TabsContent>

          {/* Project auto memory: bounded index + on-demand topic files. */}
          <TabsContent value="auto-memory" className="flex-1 overflow-hidden p-0">
            <ProjectMemorySection projectId={project.id} />
          </TabsContent>
        </Tabs>
      </SheetContent>
    </Sheet>
  )
}

function StatCard({ label, value }: { label: string; value: number }) {
  return (
    <div className="rounded-lg border border-border/60 bg-accent/20 px-3 py-3">
      <div className="text-2xl font-semibold">{value}</div>
      <div className="text-xs text-muted-foreground">{label}</div>
    </div>
  )
}
