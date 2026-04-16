/**
 * Project home dialog — tabbed view of a single project.
 *
 * Tabs: Overview | Sessions | Files | Instructions
 *
 * Keeps everything inside a Dialog so we don't have to touch the main
 * ChatScreen routing. Clicking "New session in project" closes the dialog
 * and delegates to the caller, which knows how to wire the session flow.
 */

import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { MessageSquarePlus, Pencil, Trash2, Archive, ArchiveRestore } from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Textarea } from "@/components/ui/textarea"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type {
  Project,
  ProjectMeta,
  UpdateProjectInput,
} from "@/types/project"
import type { SessionMeta } from "@/types/chat"

import ProjectFilesPanel from "./ProjectFilesPanel"

interface ProjectOverviewDialogProps {
  open: boolean
  project: ProjectMeta | null
  onOpenChange: (open: boolean) => void
  onEdit: (project: Project) => void
  onDelete: (project: Project) => void
  onArchive: (project: Project, archived: boolean) => void
  onNewSessionInProject: (projectId: string, defaultAgentId?: string | null) => void
  onOpenSession: (sessionId: string) => void
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
  onOpenSession,
  onUpdateProject,
}: ProjectOverviewDialogProps) {
  const { t } = useTranslation()
  const [tab, setTab] = useState("overview")
  const [sessions, setSessions] = useState<SessionMeta[]>([])
  const [loadingSessions, setLoadingSessions] = useState(false)
  const [instructionsDraft, setInstructionsDraft] = useState("")
  const [savingInstructions, setSavingInstructions] = useState(false)
  const [instructionsSaveStatus, setInstructionsSaveStatus] = useState<
    "idle" | "saved" | "failed"
  >("idle")

  useEffect(() => {
    if (!open || !project) return
    setTab("overview")
    setInstructionsDraft(project.instructions ?? "")
    setInstructionsSaveStatus("idle")
    void loadSessions(project.id)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, project?.id])

  async function loadSessions(pid: string) {
    setLoadingSessions(true)
    try {
      const result = await getTransport().call<[SessionMeta[], number]>(
        "list_project_sessions_cmd",
        { id: pid, limit: 50, offset: 0 },
      )
      // Tauri returns a tuple; HTTP returns `{ sessions, total }`.
      if (Array.isArray(result)) {
        setSessions(result[0] ?? [])
      } else {
        const r = result as unknown as { sessions?: SessionMeta[] }
        setSessions(r.sessions ?? [])
      }
    } catch (e) {
      logger.warn("ProjectOverviewDialog", "loadSessions failed", e)
      setSessions([])
    } finally {
      setLoadingSessions(false)
    }
  }

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
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-3xl max-h-[85vh] flex flex-col">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <span className="text-2xl">{project.emoji ?? "📁"}</span>
            <span className="flex-1 truncate">{project.name}</span>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => onEdit(project)}
              className="h-8 w-8 p-0"
              title={t("common.edit")}
            >
              <Pencil className="h-3.5 w-3.5" />
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => onArchive(project, !project.archived)}
              className="h-8 w-8 p-0 text-muted-foreground"
              title={project.archived ? t("project.unarchiveProject") : t("project.archiveProject")}
            >
              {project.archived ? (
                <ArchiveRestore className="h-3.5 w-3.5" />
              ) : (
                <Archive className="h-3.5 w-3.5" />
              )}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => onDelete(project)}
              className="h-8 w-8 p-0 text-muted-foreground hover:text-destructive"
              title={t("common.delete")}
            >
              <Trash2 className="h-3.5 w-3.5" />
            </Button>
          </DialogTitle>
          {project.description && (
            <p className="text-sm text-muted-foreground">{project.description}</p>
          )}
        </DialogHeader>

        <Tabs value={tab} onValueChange={setTab} className="flex-1 flex flex-col overflow-hidden">
          <TabsList className="shrink-0">
            <TabsTrigger value="overview">{t("project.tabOverview")}</TabsTrigger>
            <TabsTrigger value="sessions">
              {t("project.tabSessions")} · {project.sessionCount}
            </TabsTrigger>
            <TabsTrigger value="files">
              {t("project.tabFiles")} · {project.fileCount}
            </TabsTrigger>
            <TabsTrigger value="instructions">{t("project.tabInstructions")}</TabsTrigger>
          </TabsList>

          {/* Overview */}
          <TabsContent value="overview" className="flex-1 overflow-y-auto space-y-4 pt-3">
            <div className="grid grid-cols-3 gap-3">
              <StatCard
                label={t("project.overview.totalSessions")}
                value={project.sessionCount}
              />
              <StatCard
                label={t("project.overview.totalFiles")}
                value={project.fileCount}
              />
              <StatCard
                label={t("project.overview.totalMemories")}
                value={project.memoryCount}
              />
            </div>
            <Button
              onClick={() => {
                onNewSessionInProject(project.id, project.defaultAgentId)
                onOpenChange(false)
              }}
              className="w-full"
            >
              <MessageSquarePlus className="mr-2 h-4 w-4" />
              {t("project.newChatInProject")}
            </Button>
          </TabsContent>

          {/* Sessions */}
          <TabsContent value="sessions" className="flex-1 overflow-y-auto pt-3">
            {loadingSessions ? (
              <p className="text-sm text-muted-foreground text-center py-4">...</p>
            ) : sessions.length === 0 ? (
              <p className="text-sm text-muted-foreground text-center py-8">
                {t("project.sessionsInProject", { count: 0 })}
              </p>
            ) : (
              <div className="space-y-1">
                {sessions.map((s) => (
                  <button
                    key={s.id}
                    onClick={() => {
                      onOpenSession(s.id)
                      onOpenChange(false)
                    }}
                    className="w-full text-left px-3 py-2 rounded-md hover:bg-accent/40 transition-colors"
                  >
                    <div className="text-sm truncate">{s.title || "Untitled"}</div>
                    <div className="text-xs text-muted-foreground">
                      {new Date(s.updatedAt).toLocaleString()} · {s.messageCount}
                    </div>
                  </button>
                ))}
              </div>
            )}
          </TabsContent>

          {/* Files */}
          <TabsContent value="files" className="flex-1 overflow-hidden pt-3">
            <ProjectFilesPanel projectId={project.id} />
          </TabsContent>

          {/* Instructions */}
          <TabsContent
            value="instructions"
            className="flex-1 overflow-y-auto pt-3 space-y-3"
          >
            <p className="text-xs text-muted-foreground">
              {t("project.projectInstructionsHint")}
            </p>
            <Textarea
              value={instructionsDraft}
              onChange={(e) => setInstructionsDraft(e.target.value)}
              rows={10}
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
      </DialogContent>
    </Dialog>
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

