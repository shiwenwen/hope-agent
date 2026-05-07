/**
 * Project settings sheet (formerly `ProjectOverviewDialog`).
 *
 * Slides in from the right as a non-modal-feeling drawer. Tabs:
 * Overview | Files | Instructions. The old "Sessions" tab is gone — the
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
import { Textarea } from "@/components/ui/textarea"
import type { Project, ProjectMeta, UpdateProjectInput } from "@/types/project"

import ProjectFilesPanel from "./ProjectFilesPanel"
import ProjectIcon from "./ProjectIcon"

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
  onUpdateProject: (id: string, patch: UpdateProjectInput) => Promise<Project | null>
}

export default function ProjectOverviewDialog({
  open,
  project,
  onOpenChange,
  onEdit,
  onDelete,
  onArchive,
  onNewSessionInProject,
  onUpdateProject,
}: ProjectOverviewDialogProps) {
  const { t } = useTranslation()
  const [tab, setTab] = useState("overview")
  const [instructionsDraft, setInstructionsDraft] = useState("")
  const [savingInstructions, setSavingInstructions] = useState(false)
  const [instructionsSaveStatus, setInstructionsSaveStatus] = useState<"idle" | "saved" | "failed">(
    "idle",
  )

  useEffect(() => {
    if (!open || !project) return
    setTab("overview")
    setInstructionsDraft(project.instructions ?? "")
    setInstructionsSaveStatus("idle")
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, project?.id])

  async function handleSaveInstructions() {
    if (!project) return
    setSavingInstructions(true)
    try {
      const updated = await onUpdateProject(project.id, {
        instructions: instructionsDraft.trim(),
      })
      setInstructionsSaveStatus(updated ? "saved" : "failed")
    } finally {
      setSavingInstructions(false)
      setTimeout(() => setInstructionsSaveStatus("idle"), 2000)
    }
  }

  if (!project) return null

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent
        side="right"
        className="w-full sm:max-w-[560px] p-0 flex flex-col"
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
                  project.archived
                    ? t("project.unarchiveProject")
                    : t("project.archiveProject")
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
            <TabsTrigger value="files">
              {t("project.tabFiles")} · {project.fileCount}
            </TabsTrigger>
            <TabsTrigger value="instructions">{t("project.tabInstructions")}</TabsTrigger>
          </TabsList>

          {/* Overview */}
          <TabsContent value="overview" className="flex-1 overflow-y-auto px-5 py-3 space-y-4">
            <div className="grid grid-cols-3 gap-3">
              <StatCard label={t("project.overview.totalSessions")} value={project.sessionCount} />
              <StatCard label={t("project.overview.totalFiles")} value={project.fileCount} />
              <StatCard label={t("project.overview.totalMemories")} value={project.memoryCount} />
            </div>

            <Button
              onClick={() => {
                onNewSessionInProject(project.id, project.defaultAgentId)
                onOpenChange(false)
              }}
              className="w-full"
            >
              {t("project.newChatInProject")}
            </Button>
          </TabsContent>

          {/* Files */}
          <TabsContent value="files" className="flex-1 overflow-hidden px-5 py-3">
            <ProjectFilesPanel projectId={project.id} />
          </TabsContent>

          {/* Instructions */}
          <TabsContent
            value="instructions"
            className="flex-1 overflow-y-auto px-5 py-3 space-y-3"
          >
            <p className="text-xs text-muted-foreground">{t("project.projectInstructionsHint")}</p>
            <Textarea
              value={instructionsDraft}
              onChange={(e) => setInstructionsDraft(e.target.value)}
              rows={12}
              className="font-mono text-sm"
              placeholder={t("project.projectInstructionsPlaceholder")}
            />
            <div className="flex justify-end gap-2">
              <Button
                variant="outline"
                onClick={() => setInstructionsDraft(project.instructions ?? "")}
                disabled={savingInstructions}
              >
                {t("common.cancel")}
              </Button>
              <Button
                onClick={handleSaveInstructions}
                disabled={savingInstructions}
                className={
                  instructionsSaveStatus === "saved"
                    ? "bg-emerald-600 hover:bg-emerald-600"
                    : instructionsSaveStatus === "failed"
                      ? "bg-destructive hover:bg-destructive"
                      : ""
                }
              >
                {savingInstructions
                  ? t("common.saving")
                  : instructionsSaveStatus === "saved"
                    ? t("common.saved")
                    : t("common.save")}
              </Button>
            </div>
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
