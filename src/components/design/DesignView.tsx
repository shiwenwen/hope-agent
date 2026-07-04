/**
 * 设计空间独立视图（侧边栏入口）。
 *
 * 形态：首页（项目墙）↔ 工作室（产物库 + 单产物稳定预览）。
 * 刻意**不做无限画布**——多产物概览用纯 CSS grid 缩略图墙，单产物聚焦用一个
 * 稳定 iframe + CSS 缩放，从架构上规避旧版画布卡顿。见 docs/architecture/design-space.md。
 */

import { useCallback, useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  ArrowLeft,
  Plus,
  Trash2,
  RefreshCw,
  Settings2,
  Palette,
  Loader2,
  Monitor,
  Smartphone,
  Presentation,
  LayoutDashboard,
  Image as ImageIcon,
  FileText,
  Mail,
  Sparkles,
} from "lucide-react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { IconTip } from "@/components/ui/tooltip"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog"
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
} from "@/components/ui/dropdown-menu"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import type {
  ArtifactKind,
  DesignArtifact,
  DesignArtifactView,
  DesignProject,
} from "@/types/design"
import { ARTIFACT_KINDS } from "@/types/design"

interface DesignViewProps {
  onBack: () => void
  onOpenSettings: () => void
}

const KIND_ICON: Record<ArtifactKind, typeof Monitor> = {
  web: Monitor,
  mobile: Smartphone,
  deck: Presentation,
  dashboard: LayoutDashboard,
  poster: ImageIcon,
  document: FileText,
  email: Mail,
  image: Sparkles,
}

type ZoomMode = "fit" | 0.5 | 1

export default function DesignView({ onBack, onOpenSettings }: DesignViewProps) {
  const { t } = useTranslation()
  const tx = getTransport()

  const [projects, setProjects] = useState<DesignProject[]>([])
  const [activeProject, setActiveProject] = useState<DesignProject | null>(null)
  const [artifacts, setArtifacts] = useState<DesignArtifact[]>([])
  const [activeArtifact, setActiveArtifact] = useState<DesignArtifactView | null>(null)
  const [loadingProjects, setLoadingProjects] = useState(false)
  const [loadingArtifacts, setLoadingArtifacts] = useState(false)

  const [newProjectOpen, setNewProjectOpen] = useState(false)
  const [newProjectTitle, setNewProjectTitle] = useState("")
  const [creatingProject, setCreatingProject] = useState(false)

  const [deleteTarget, setDeleteTarget] = useState<
    { type: "project"; id: string; title: string } | { type: "artifact"; id: string; title: string } | null
  >(null)

  const [zoom, setZoom] = useState<ZoomMode>("fit")
  const [previewKey, setPreviewKey] = useState(0)
  const iframeRef = useRef<HTMLIFrameElement>(null)

  const kindLabel = useCallback(
    (kind: ArtifactKind) => t(`design.kind.${kind}`, kind),
    [t],
  )

  // ── Projects ─────────────────────────────────────────────────

  const loadProjects = useCallback(async () => {
    setLoadingProjects(true)
    try {
      const list = await tx.call<DesignProject[]>("list_design_projects_cmd")
      setProjects(list ?? [])
    } catch (e) {
      logger.error("design", "DesignView::loadProjects", "list projects failed", e)
    } finally {
      setLoadingProjects(false)
    }
  }, [tx])

  useEffect(() => {
    void loadProjects()
  }, [loadProjects])

  const createProject = useCallback(async () => {
    setCreatingProject(true)
    try {
      const project = await tx.call<DesignProject>("create_design_project_cmd", {
        input: { title: newProjectTitle.trim() || t("design.untitledProject", "未命名项目") },
      })
      setNewProjectOpen(false)
      setNewProjectTitle("")
      await loadProjects()
      if (project) openProject(project)
    } catch (e) {
      logger.error("design", "DesignView::createProject", "create project failed", e)
    } finally {
      setCreatingProject(false)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tx, newProjectTitle, t, loadProjects])

  // ── Artifacts ────────────────────────────────────────────────

  const loadArtifacts = useCallback(
    async (projectId: string) => {
      setLoadingArtifacts(true)
      try {
        const list = await tx.call<DesignArtifact[]>("list_design_artifacts_cmd", {
          projectId,
        })
        setArtifacts(list ?? [])
      } catch (e) {
        logger.error("design", "DesignView::loadArtifacts", "list artifacts failed", e)
      } finally {
        setLoadingArtifacts(false)
      }
    },
    [tx],
  )

  const openProject = useCallback(
    (project: DesignProject) => {
      setActiveProject(project)
      setActiveArtifact(null)
      void loadArtifacts(project.id)
    },
    [loadArtifacts],
  )

  const backToHome = useCallback(() => {
    setActiveProject(null)
    setActiveArtifact(null)
    setArtifacts([])
    void loadProjects()
  }, [loadProjects])

  const openArtifact = useCallback(
    async (artifact: DesignArtifact) => {
      try {
        const view = await tx.call<DesignArtifactView | null>("get_design_artifact_cmd", {
          id: artifact.id,
        })
        if (view) {
          setActiveArtifact(view)
          setPreviewKey((k) => k + 1)
        }
      } catch (e) {
        logger.error("design", "DesignView::openArtifact", "open artifact failed", e)
      }
    },
    [tx],
  )

  const createArtifact = useCallback(
    async (kind: ArtifactKind) => {
      if (!activeProject) return
      try {
        const artifact = await tx.call<DesignArtifact>("create_design_artifact_cmd", {
          input: {
            projectId: activeProject.id,
            title: `${kindLabel(kind)}`,
            kind,
          },
        })
        await loadArtifacts(activeProject.id)
        if (artifact) void openArtifact(artifact)
      } catch (e) {
        logger.error("design", "DesignView::createArtifact", "create artifact failed", e)
      }
    },
    [tx, activeProject, kindLabel, loadArtifacts, openArtifact],
  )

  // ── Delete (shared confirm) ──────────────────────────────────

  const confirmDelete = useCallback(async () => {
    if (!deleteTarget) return
    try {
      if (deleteTarget.type === "project") {
        await tx.call("delete_design_project_cmd", { id: deleteTarget.id })
        if (activeProject?.id === deleteTarget.id) backToHome()
        await loadProjects()
      } else {
        await tx.call("delete_design_artifact_cmd", { id: deleteTarget.id })
        if (activeArtifact?.id === deleteTarget.id) setActiveArtifact(null)
        if (activeProject) await loadArtifacts(activeProject.id)
      }
    } catch (e) {
      logger.error("design", "DesignView::confirmDelete", "delete failed", e)
    } finally {
      setDeleteTarget(null)
    }
  }, [deleteTarget, tx, activeProject, activeArtifact, backToHome, loadProjects, loadArtifacts])

  // ── Live events ──────────────────────────────────────────────

  useEffect(() => {
    const off = [
      tx.listen("design:artifact_ready", () => {
        if (activeProject) void loadArtifacts(activeProject.id)
      }),
      tx.listen("design:artifact_deleted", () => {
        if (activeProject) void loadArtifacts(activeProject.id)
      }),
      tx.listen("design:reload", () => {
        setPreviewKey((k) => k + 1)
      }),
      tx.listen("design:project_changed", () => {
        if (!activeProject) void loadProjects()
      }),
    ]
    return () => off.forEach((f) => f())
  }, [tx, activeProject, loadArtifacts, loadProjects])

  // ── Preview iframe src ───────────────────────────────────────

  const iframeSrc = activeArtifact
    ? tx.resolveAssetUrl(`${activeArtifact.artifactPath}/index.html`) ?? ""
    : ""

  const scaleStyle =
    zoom === "fit"
      ? { width: "100%", height: "100%" }
      : {
          width: `${100 / zoom}%`,
          height: `${100 / zoom}%`,
          transform: `scale(${zoom})`,
          transformOrigin: "top left" as const,
        }

  // ── Render ───────────────────────────────────────────────────

  return (
    <div className="flex flex-1 min-h-0 min-w-0 flex-col bg-background">
      {/* Header */}
      <header
        className="flex h-12 shrink-0 items-center gap-2 border-b px-3"
        data-tauri-drag-region
      >
        {activeProject ? (
          <IconTip label={t("design.backToProjects", "返回项目")} side="bottom">
            <Button variant="ghost" size="icon" className="h-8 w-8" onClick={backToHome}>
              <ArrowLeft className="h-4 w-4" />
            </Button>
          </IconTip>
        ) : (
          <IconTip label={t("common.back", "返回")} side="bottom">
            <Button variant="ghost" size="icon" className="h-8 w-8" onClick={onBack}>
              <ArrowLeft className="h-4 w-4" />
            </Button>
          </IconTip>
        )}
        <Palette className="h-4 w-4 text-primary" />
        <span className="text-sm font-semibold">
          {activeProject ? activeProject.title : t("design.title", "设计空间")}
        </span>
        <div className="ml-auto flex items-center gap-1">
          {activeProject && (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button size="sm" className="h-8 gap-1.5">
                  <Plus className="h-4 w-4" />
                  {t("design.newArtifact", "新建产物")}
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                {ARTIFACT_KINDS.map((kind) => {
                  const Icon = KIND_ICON[kind]
                  return (
                    <DropdownMenuItem key={kind} onSelect={() => void createArtifact(kind)}>
                      <Icon className="mr-2 h-4 w-4" />
                      {kindLabel(kind)}
                    </DropdownMenuItem>
                  )
                })}
              </DropdownMenuContent>
            </DropdownMenu>
          )}
          <IconTip label={t("common.settings", "设置")} side="bottom">
            <Button variant="ghost" size="icon" className="h-8 w-8" onClick={onOpenSettings}>
              <Settings2 className="h-4 w-4" />
            </Button>
          </IconTip>
        </div>
      </header>

      {/* Body */}
      {!activeProject ? (
        <HomeGrid
          projects={projects}
          loading={loadingProjects}
          onNew={() => setNewProjectOpen(true)}
          onOpen={openProject}
          onDelete={(p) => setDeleteTarget({ type: "project", id: p.id, title: p.title })}
        />
      ) : (
        <div className="flex flex-1 min-h-0">
          {/* Artifact library (left) */}
          <aside className="w-72 shrink-0 overflow-y-auto border-r p-3">
            {loadingArtifacts ? (
              <div className="flex justify-center py-8">
                <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
              </div>
            ) : artifacts.length === 0 ? (
              <div className="px-2 py-8 text-center text-sm text-muted-foreground">
                {t("design.emptyArtifacts", "还没有产物。点右上角「新建产物」开始。")}
              </div>
            ) : (
              <ul className="space-y-1.5">
                {artifacts.map((a) => {
                  const Icon = KIND_ICON[a.kind] ?? Monitor
                  const active = activeArtifact?.id === a.id
                  return (
                    <li key={a.id}>
                      <button
                        type="button"
                        onClick={() => void openArtifact(a)}
                        className={cn(
                          "group flex w-full items-center gap-2 rounded-lg px-2.5 py-2 text-left text-sm transition-colors",
                          active
                            ? "bg-primary/10 text-primary"
                            : "hover:bg-muted text-foreground",
                        )}
                      >
                        <Icon className="h-4 w-4 shrink-0 opacity-70" />
                        <span className="min-w-0 flex-1 truncate">{a.title}</span>
                        <span
                          role="button"
                          tabIndex={0}
                          onClick={(e) => {
                            e.stopPropagation()
                            setDeleteTarget({ type: "artifact", id: a.id, title: a.title })
                          }}
                          className="opacity-0 group-hover:opacity-100 text-muted-foreground hover:text-destructive transition-opacity"
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </span>
                      </button>
                    </li>
                  )
                })}
              </ul>
            )}
          </aside>

          {/* Single-artifact preview (center) */}
          <main className="flex flex-1 min-w-0 flex-col bg-muted/30">
            {activeArtifact ? (
              <>
                <div className="flex h-9 shrink-0 items-center gap-2 border-b bg-background/60 px-3">
                  <span className="truncate text-xs font-medium text-muted-foreground">
                    {activeArtifact.title}
                  </span>
                  <div className="ml-auto flex items-center gap-1">
                    <select
                      value={String(zoom)}
                      onChange={(e) => {
                        const v = e.target.value
                        setZoom(v === "fit" ? "fit" : (Number(v) as ZoomMode))
                      }}
                      className="h-6 rounded border bg-background px-1.5 text-xs"
                    >
                      <option value="fit">{t("design.zoomFit", "适应")}</option>
                      <option value="1">100%</option>
                      <option value="0.5">50%</option>
                    </select>
                    <IconTip label={t("design.reload", "刷新")} side="bottom">
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-6 w-6"
                        onClick={() => setPreviewKey((k) => k + 1)}
                      >
                        <RefreshCw className="h-3.5 w-3.5" />
                      </Button>
                    </IconTip>
                  </div>
                </div>
                <div className="flex-1 overflow-auto p-4">
                  <div className="mx-auto h-full w-full overflow-hidden rounded-lg border bg-white shadow-sm">
                    <iframe
                      ref={iframeRef}
                      key={`${activeArtifact.id}-${previewKey}`}
                      src={iframeSrc}
                      sandbox="allow-scripts"
                      title={activeArtifact.title}
                      className="border-0"
                      style={scaleStyle}
                    />
                  </div>
                </div>
              </>
            ) : (
              <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
                {t("design.selectArtifact", "从左侧选择一个产物预览")}
              </div>
            )}
          </main>
        </div>
      )}

      {/* New project dialog */}
      <Dialog open={newProjectOpen} onOpenChange={setNewProjectOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("design.newProject", "新建设计项目")}</DialogTitle>
          </DialogHeader>
          <Input
            autoFocus
            value={newProjectTitle}
            onChange={(e) => setNewProjectTitle(e.target.value)}
            placeholder={t("design.projectTitlePlaceholder", "项目名称")}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !creatingProject) void createProject()
            }}
          />
          <DialogFooter>
            <Button variant="ghost" onClick={() => setNewProjectOpen(false)}>
              {t("common.cancel", "取消")}
            </Button>
            <Button onClick={() => void createProject()} disabled={creatingProject}>
              {creatingProject && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
              {t("common.create", "创建")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Delete confirm */}
      <AlertDialog open={!!deleteTarget} onOpenChange={(o) => !o && setDeleteTarget(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("design.deleteTitle", "确认删除？")}</AlertDialogTitle>
            <AlertDialogDescription>
              {deleteTarget?.type === "project"
                ? t("design.deleteProjectDesc", "将永久删除该项目及其全部产物，无法恢复。")
                : t("design.deleteArtifactDesc", "将永久删除该产物及其全部版本，无法恢复。")}
              {deleteTarget ? `（${deleteTarget.title}）` : ""}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel", "取消")}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => void confirmDelete()}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            >
              {t("common.delete", "删除")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

// ── Home project wall ──────────────────────────────────────────

function HomeGrid({
  projects,
  loading,
  onNew,
  onOpen,
  onDelete,
}: {
  projects: DesignProject[]
  loading: boolean
  onNew: () => void
  onOpen: (p: DesignProject) => void
  onDelete: (p: DesignProject) => void
}) {
  const { t } = useTranslation()
  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="mx-auto max-w-5xl">
        <div className="mb-4 flex items-center justify-between">
          <div>
            <h2 className="text-lg font-semibold">{t("design.projects", "设计项目")}</h2>
            <p className="text-sm text-muted-foreground">
              {t("design.projectsSub", "你的第二设计大脑：从一句话产出可交付的设计。")}
            </p>
          </div>
          <Button onClick={onNew} className="gap-1.5">
            <Plus className="h-4 w-4" />
            {t("design.newProject", "新建项目")}
          </Button>
        </div>

        {loading ? (
          <div className="flex justify-center py-16">
            <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
          </div>
        ) : projects.length === 0 ? (
          <button
            type="button"
            onClick={onNew}
            className="flex min-h-[180px] w-full flex-col items-center justify-center gap-2 rounded-xl border-2 border-dashed text-muted-foreground transition-colors hover:border-primary/40 hover:text-foreground"
          >
            <Palette className="h-8 w-8 opacity-40" />
            <span className="text-sm">{t("design.emptyProjects", "还没有设计项目，点此创建第一个")}</span>
          </button>
        ) : (
          <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-4">
            {projects.map((p) => (
              <div
                key={p.id}
                className="group relative flex cursor-pointer flex-col overflow-hidden rounded-xl border bg-card transition-shadow hover:shadow-md"
                onClick={() => onOpen(p)}
              >
                <div
                  className="flex aspect-[4/3] items-center justify-center bg-gradient-to-br from-muted to-muted/40"
                  style={p.color ? { background: p.color } : undefined}
                >
                  <Palette className="h-8 w-8 text-muted-foreground/40" />
                </div>
                <div className="flex items-center gap-2 p-3">
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-sm font-medium">{p.title}</div>
                    <div className="text-xs text-muted-foreground">
                      {t("design.artifactCount", "{{count}} 个产物").replace(
                        "{{count}}",
                        String(p.artifactCount ?? 0),
                      )}
                    </div>
                  </div>
                  <span
                    role="button"
                    tabIndex={0}
                    onClick={(e) => {
                      e.stopPropagation()
                      onDelete(p)
                    }}
                    className="opacity-0 group-hover:opacity-100 text-muted-foreground hover:text-destructive transition-opacity"
                  >
                    <Trash2 className="h-4 w-4" />
                  </span>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}
