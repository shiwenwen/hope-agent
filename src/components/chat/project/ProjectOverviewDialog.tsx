/**
 * Project settings sheet (formerly `ProjectOverviewDialog`).
 *
 * Slides in from the right as a non-modal-feeling drawer. Tabs:
 * Overview | Files | Instructions. The old "Sessions" tab is gone — the
 * sidebar now renders project sessions inline as a nested tree node, so
 * having the same list inside this sheet is redundant.
 *
 * The Overview tab also surfaces the IM-channel binding for this project.
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Textarea } from "@/components/ui/textarea"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { ChannelAccountConfig } from "@/components/settings/channel-panel/types"
import type { Project, ProjectMeta, UpdateProjectInput } from "@/types/project"

import ProjectFilesPanel from "./ProjectFilesPanel"
import ProjectIcon from "./ProjectIcon"

/** Sentinel value for "unbound" in the Radix Select — empty strings are
 *  rejected by Radix, so we map None ↔ this constant at the boundary. */
const UNBOUND_SENTINEL = "__none__"

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
  const [channels, setChannels] = useState<ChannelAccountConfig[]>([])
  const [savingChannel, setSavingChannel] = useState(false)

  useEffect(() => {
    if (!open || !project) return
    setTab("overview")
    setInstructionsDraft(project.instructions ?? "")
    setInstructionsSaveStatus("idle")
    void loadChannels()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, project?.id])

  async function loadChannels() {
    try {
      const accounts = await getTransport().call<ChannelAccountConfig[]>("channel_list_accounts")
      setChannels(accounts ?? [])
    } catch (e) {
      logger.warn("chat", "ProjectOverviewDialog", "loadChannels failed", e)
      setChannels([])
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

  async function handleSaveBoundChannel(value: { channelId: string; accountId: string } | null) {
    if (!project) return
    setSavingChannel(true)
    try {
      // Patch boundChannel: `null` clears, an object sets. The backend
      // double-Option pattern interprets these distinctly.
      await onUpdateProject(project.id, { boundChannel: value })
    } finally {
      setSavingChannel(false)
    }
  }

  const boundChannelLabel = (() => {
    if (!project?.boundChannel) return null
    const acc = channels.find(
      (c) =>
        c.id === project.boundChannel?.accountId && c.channelId === project.boundChannel?.channelId,
    )
    return acc?.label ?? `${project.boundChannel.channelId} / ${project.boundChannel.accountId}`
  })()

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

            {/* Bound IM channel */}
            <div className="rounded-lg border border-border/60 bg-muted/30 p-3 space-y-2">
              <div className="flex items-center justify-between">
                <div className="text-xs font-semibold uppercase tracking-wider text-muted-foreground/80">
                  {t("project.bindChannelLabel")}
                </div>
                {project.boundChannel && (
                  <button
                    onClick={() => handleSaveBoundChannel(null)}
                    disabled={savingChannel}
                    className="text-[11px] text-muted-foreground hover:text-destructive transition-colors"
                  >
                    {t("project.unbindChannel")}
                  </button>
                )}
              </div>
              <p className="text-[11px] text-muted-foreground">{t("project.bindChannelHelp")}</p>
              <Select
                value={project.boundChannel?.accountId ?? UNBOUND_SENTINEL}
                disabled={savingChannel}
                onValueChange={(v) => {
                  if (v === UNBOUND_SENTINEL) {
                    void handleSaveBoundChannel(null)
                    return
                  }
                  const account = channels.find((c) => c.id === v)
                  if (account) {
                    void handleSaveBoundChannel({
                      channelId: account.channelId,
                      accountId: account.id,
                    })
                  }
                }}
              >
                <SelectTrigger className="w-full h-8 text-sm">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={UNBOUND_SENTINEL}>— {t("project.unbindChannel")} —</SelectItem>
                  {channels.map((c) => (
                    <SelectItem key={c.id} value={c.id}>
                      {c.label} ({c.channelId})
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {boundChannelLabel && (
                <div className="text-[11px] text-emerald-600 dark:text-emerald-400">
                  ✓ {t("project.boundChannel")}: {boundChannelLabel}
                </div>
              )}
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
          <TabsContent value="instructions" className="flex-1 overflow-y-auto px-5 py-3 space-y-3">
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
