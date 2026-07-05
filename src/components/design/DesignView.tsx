/**
 * 设计空间独立视图（侧边栏入口）。
 *
 * 形态：首页（项目墙）↔ 工作室（产物库 + 单产物稳定预览）。
 * 刻意**不做无限画布**——多产物概览用纯 CSS grid 缩略图墙，单产物聚焦用一个
 * 稳定 iframe + CSS 缩放，从架构上规避旧版画布卡顿。见 docs/architecture/design-space.md。
 */

import { useCallback, useEffect, useRef, useState } from "react"
import type { CSSProperties } from "react"
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
  MousePointerClick,
  Download,
  Gauge,
  Film,
  History,
  Wand2,
  RotateCcw,
  FileImage,
  FileType2,
  FileArchive,
  FileCode,
  Code2,
  AlertCircle,
  X,
  Loader2 as Loader2Icon,
} from "lucide-react"
import { toast } from "sonner"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload } from "@/lib/transport"
import DesignInspector from "@/components/design/DesignInspector"
import { DesignSystemPicker } from "@/components/design/DesignSystemPicker"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
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
  DropdownMenuSeparator,
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
  DesignArtifactVersion,
  DesignArtifactView,
  DesignProject,
  DesignSystemMeta,
  DesignSelectedElement,
  DesignDirection,
  DesignConfig,
  CritiqueResult,
} from "@/types/design"
import { ARTIFACT_KINDS } from "@/types/design"
import {
  exportPng,
  exportPdf,
  exportPptx,
  downloadBlob,
  base64ToBlob,
  safeFilename,
} from "@/lib/designExport"
import { exportVideo, videoExportSupported } from "@/lib/designVideo"

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
  motion: Film,
}

type ZoomMode = "fit" | 0.5 | 1

export default function DesignView({ onBack, onOpenSettings }: DesignViewProps) {
  const { t } = useTranslation()
  const tx = getTransport()

  const [projects, setProjects] = useState<DesignProject[]>([])
  const [systems, setSystems] = useState<DesignSystemMeta[]>([])
  const [activeProject, setActiveProject] = useState<DesignProject | null>(null)
  const [artifacts, setArtifacts] = useState<DesignArtifact[]>([])
  const [activeArtifact, setActiveArtifact] = useState<DesignArtifactView | null>(null)
  const [loadingProjects, setLoadingProjects] = useState(false)
  const [loadingArtifacts, setLoadingArtifacts] = useState(false)

  const [newProjectOpen, setNewProjectOpen] = useState(false)
  const [newProjectTitle, setNewProjectTitle] = useState("")
  const [creatingProject, setCreatingProject] = useState(false)
  const [systemPickerOpen, setSystemPickerOpen] = useState(false)

  const [deleteTarget, setDeleteTarget] = useState<
    { type: "project"; id: string; title: string } | { type: "artifact"; id: string; title: string } | null
  >(null)

  const [zoom, setZoom] = useState<ZoomMode>("fit")
  const [previewKey, setPreviewKey] = useState(0)
  const iframeRef = useRef<HTMLIFrameElement>(null)

  // 可视化微调（D1）
  const [editMode, setEditMode] = useState(false)
  const [selected, setSelected] = useState<DesignSelectedElement | null>(null)
  const selectedRef = useRef<DesignSelectedElement | null>(null)
  selectedRef.current = selected
  const editModeRef = useRef(false)
  editModeRef.current = editMode
  // Live refs so the EventBus subscription can read current project/artifact without
  // being a dependency (avoids re-subscribing — and dropping events — on every edit).
  const activeProjectRef = useRef<DesignProject | null>(null)
  activeProjectRef.current = activeProject
  const activeArtifactRef = useRef<DesignArtifactView | null>(null)
  activeArtifactRef.current = activeArtifact

  const postToIframe = useCallback((msg: Record<string, unknown>) => {
    iframeRef.current?.contentWindow?.postMessage(msg, "*")
  }, [])

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
      toast.error(t("design.err.load", "加载失败"))
    } finally {
      setLoadingProjects(false)
    }
  }, [tx, t])

  useEffect(() => {
    void loadProjects()
  }, [loadProjects])

  const loadSystems = useCallback(async () => {
    try {
      const list = await tx.call<DesignSystemMeta[]>("list_design_systems_cmd")
      setSystems(list ?? [])
    } catch (e) {
      logger.error("design", "DesignView::loadSystems", "list systems failed", e)
      toast.error(t("design.err.load", "加载失败"))
    }
  }, [tx, t])

  useEffect(() => {
    void loadSystems()
  }, [loadSystems])

  // Export clarity/quality prefs (config-driven; undefined → export defaults).
  const [designConfig, setDesignConfig] = useState<DesignConfig | null>(null)
  useEffect(() => {
    tx.call<DesignConfig>("get_design_config_cmd")
      .then(setDesignConfig)
      .catch(() => {})
  }, [tx])

  const setProjectSystem = useCallback(
    async (systemId: string | null) => {
      if (!activeProject) return
      try {
        const updated = await tx.call<DesignProject>("update_design_project_cmd", {
          input: { id: activeProject.id, defaultSystemId: systemId ?? "" },
        })
        if (updated) setActiveProject(updated)
      } catch (e) {
        logger.error("design", "DesignView::setProjectSystem", "set system failed", e)
        toast.error(t("design.err.setSystem", "设置设计系统失败"))
      }
    },
    [tx, activeProject, t],
  )

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
      toast.error(t("design.err.create", "创建失败"))
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
        toast.error(t("design.err.load", "加载失败"))
      } finally {
        setLoadingArtifacts(false)
      }
    },
    [tx, t],
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
        toast.error(t("design.err.load", "加载失败"))
      }
    },
    [tx, t],
  )

  const createArtifact = useCallback(
    async (kind: ArtifactKind, prompt?: string) => {
      if (!activeProject) return
      try {
        const artifact = await tx.call<DesignArtifact>("create_design_artifact_cmd", {
          input: {
            projectId: activeProject.id,
            title: kind === "image" && prompt ? prompt.slice(0, 40) : `${kindLabel(kind)}`,
            kind,
            prompt,
          },
        })
        await loadArtifacts(activeProject.id)
        if (artifact) void openArtifact(artifact)
      } catch (e) {
        logger.error("design", "DesignView::createArtifact", "create artifact failed", e)
        toast.error(
          t(
            kind === "image" ? "design.err.imageGen" : "design.err.create",
            kind === "image" ? "图像生成失败，请重试" : "创建失败",
          ),
        )
        throw e // let image-prompt flow keep its dialog open on failure
      }
    },
    [tx, activeProject, kindLabel, loadArtifacts, openArtifact, t],
  )

  // image 形态需要描述 prompt → 弹小对话框收集。
  const [imagePromptOpen, setImagePromptOpen] = useState(false)
  const [imagePrompt, setImagePrompt] = useState("")
  const [creatingImage, setCreatingImage] = useState(false)
  const onPickKind = useCallback(
    (kind: ArtifactKind) => {
      if (kind === "image") {
        setImagePrompt("")
        setImagePromptOpen(true)
      } else {
        // error already surfaced via toast in createArtifact; swallow the rejection
        void createArtifact(kind).catch(() => {})
      }
    },
    [createArtifact],
  )
  const confirmImagePrompt = useCallback(async () => {
    if (!imagePrompt.trim()) return
    setCreatingImage(true)
    try {
      await createArtifact("image", imagePrompt.trim())
      setImagePromptOpen(false) // only on success — createArtifact throws on failure
    } catch {
      // error already surfaced via toast in createArtifact; keep dialog open to retry
    } finally {
      setCreatingImage(false)
    }
  }, [createArtifact, imagePrompt])

  // ── Prompt-first launch (home hero → generate) ───────────────

  const [homePrompt, setHomePrompt] = useState("")
  const [homeKind, setHomeKind] = useState<ArtifactKind>("web")
  const [homeSystemId, setHomeSystemId] = useState<string | null>(null)
  const [generatingHome, setGeneratingHome] = useState(false)

  // 首屏「一句话 → 生成」：建项目 → 带 prompt 建产物（后端一次模型生成完整自包含设计）→ 打开。
  const generateFromHome = useCallback(async () => {
    const prompt = homePrompt.trim()
    if (!prompt || generatingHome) return
    const systemId = homeSystemId ?? designConfig?.defaultSystemId ?? undefined
    let createdProjectId: string | null = null
    setGeneratingHome(true)
    try {
      const project = await tx.call<DesignProject>("create_design_project_cmd", {
        input: { title: prompt.slice(0, 40) },
      })
      createdProjectId = project.id
      const artifact = await tx.call<DesignArtifact>("create_design_artifact_cmd", {
        input: {
          projectId: project.id,
          title: kindLabel(homeKind),
          kind: homeKind,
          prompt,
          systemId,
        },
      })
      setHomePrompt("")
      openProject(project)
      if (artifact) void openArtifact(artifact)
    } catch (e) {
      logger.error("design", "DesignView::generateFromHome", "generate failed", e)
      toast.error(t("design.err.create", "创建失败"))
      // 回滚：产物没建成，删掉刚建的孤儿空项目（否则每次重试堆积隐藏空项目）。
      if (createdProjectId) {
        try {
          await tx.call("delete_design_project_cmd", { id: createdProjectId })
        } catch {
          /* best effort */
        }
      }
    } finally {
      setGeneratingHome(false)
    }
  }, [
    tx,
    homePrompt,
    homeKind,
    homeSystemId,
    generatingHome,
    designConfig,
    kindLabel,
    openProject,
    openArtifact,
    t,
  ])

  // ── Visual fine-tuning (D1) ──────────────────────────────────

  const suppressReloadRef = useRef(false)

  const refreshView = useCallback(async () => {
    const active = activeArtifactRef.current
    if (!active) return
    try {
      const view = await tx.call<DesignArtifactView | null>("get_design_artifact_cmd", {
        id: active.id,
      })
      if (view) setActiveArtifact(view)
    } catch {
      /* non-fatal */
    }
  }, [tx])

  const commitPatch = useCallback(
    async (patch: { oid: number; styles?: [string, string][]; text?: string }) => {
      if (!activeArtifact) return
      suppressReloadRef.current = true
      try {
        await tx.call("patch_design_element_cmd", {
          input: {
            artifactId: activeArtifact.id,
            expectedHash: activeArtifact.bodyHash,
            ...patch,
          },
        })
        await refreshView()
      } catch (e) {
        // stale write or error → hard reload to resync; clear the now-invalid
        // selection and tell the user to re-pick (oid may no longer match).
        suppressReloadRef.current = false
        setPreviewKey((k) => k + 1)
        setSelected(null)
        logger.error("design", "DesignView::commitPatch", "patch failed", e)
        toast.error(t("design.staleReselect", "源已更新，请重新选择元素后再试"))
      }
    },
    [tx, activeArtifact, refreshView, t],
  )

  const handleLiveStyle = useCallback(
    (prop: string, value: string) => {
      const oid = selectedRef.current?.oid
      if (oid == null) return
      postToIframe({ type: "ds_preview_style", oid, props: [[prop, value]] })
    },
    [postToIframe],
  )
  const handleCommitStyle = useCallback(
    (prop: string, value: string) => {
      const oid = selectedRef.current?.oid
      if (oid == null) return
      void commitPatch({ oid: Number(oid), styles: [[prop, value]] })
    },
    [commitPatch],
  )
  const handleLiveText = useCallback(
    (text: string) => {
      const oid = selectedRef.current?.oid
      if (oid == null) return
      postToIframe({ type: "ds_set_text", oid, text })
    },
    [postToIframe],
  )
  const handleCommitText = useCallback(
    (text: string) => {
      const oid = selectedRef.current?.oid
      if (oid == null) return
      void commitPatch({ oid: Number(oid), text })
    },
    [commitPatch],
  )

  // Receive selection from the iframe bridge.
  useEffect(() => {
    const onMsg = (e: MessageEvent) => {
      const d = e.data as { type?: string; payload?: DesignSelectedElement }
      if (d?.type === "ds_selected" && d.payload) setSelected(d.payload)
    }
    window.addEventListener("message", onMsg)
    return () => window.removeEventListener("message", onMsg)
  }, [])

  // Toggle bridge activation with edit mode.
  useEffect(() => {
    postToIframe({ type: editMode ? "ds_activate" : "ds_deactivate" })
    if (!editMode) setSelected(null)
  }, [editMode, postToIframe])

  // Reset edit state when switching artifacts.
  useEffect(() => {
    setEditMode(false)
    setSelected(null)
  }, [activeArtifact?.id])

  // Re-arm bridge + restore selection after an iframe (re)mount.
  const handleIframeLoad = useCallback(() => {
    if (editModeRef.current) postToIframe({ type: "ds_activate" })
    const oid = selectedRef.current?.oid
    if (oid != null) postToIframe({ type: "ds_reselect", oid })
  }, [postToIframe])

  // ── Export (D3): HTML/MD/ZIP（后端）+ PNG/PDF/PPTX/MP4（客户端栅格化） ──
  type ExportFormat = "html" | "md" | "zip" | "png" | "pdf" | "pptx" | "video"
  const [exporting, setExporting] = useState<null | ExportFormat>(null)
  const handleExport = useCallback(
    async (format: ExportFormat) => {
      if (!activeArtifact || exporting) return
      setExporting(format)
      // Text/backend formats are quick; rasterized ones can take seconds → live toast.
      const quick = format === "html" || format === "md"
      const toastId = quick ? undefined : toast.loading(t("design.exporting", "正在导出…"))
      const onProgress =
        toastId !== undefined
          ? (done: number, total: number) => {
              if (total > 1) {
                toast.loading(
                  t("design.exportProgressSlide", "正在导出 {{done}}/{{total}}", { done, total }),
                  { id: toastId },
                )
              }
            }
          : undefined
      try {
        const base = safeFilename(activeArtifact.title)
        // Text formats (HTML / Markdown) — backend returns the content directly.
        if (format === "html" || format === "md") {
          const fmt = format === "md" ? "markdown" : "html"
          const res = await tx.call<{ filename: string; mime: string; content: string }>(
            "export_design_artifact_cmd",
            { id: activeArtifact.id, format: fmt },
          )
          if (!res) return
          downloadBlob(new Blob([res.content], { type: res.mime }), res.filename || `${base}.${format}`)
          return
        }
        // ZIP — backend assembles a source bundle (base64).
        if (format === "zip") {
          const res = await tx.call<{ zip: string }>("export_design_zip_cmd", {
            artifactId: activeArtifact.id,
          })
          if (!res?.zip) return
          downloadBlob(base64ToBlob(res.zip, "application/zip"), `${base}.zip`)
          if (toastId !== undefined) toast.success(t("design.ok.exported", "已导出"), { id: toastId })
          return
        }
        // Rasterized formats (PNG/PDF/PPTX/MP4) need the clean self-contained HTML.
        const res = await tx.call<{ filename: string; mime: string; content: string }>(
          "export_design_artifact_cmd",
          { id: activeArtifact.id, format: "html" },
        )
        if (!res) return
        const kind = activeArtifact.kind
        const vw = activeArtifact.viewportW
        // Clarity/quality from config (undefined → export defaults 2x / q92).
        const exportOpts = {
          scale: designConfig?.exportScale,
          jpegQuality: designConfig?.exportJpegQuality,
          onProgress,
        }
        if (format === "png") {
          downloadBlob(await exportPng(res.content, kind, vw, exportOpts), `${base}.png`)
        } else if (format === "pdf") {
          downloadBlob(await exportPdf(res.content, kind, vw, exportOpts), `${base}.pdf`)
        } else if (format === "pptx") {
          downloadBlob(
            await exportPptx(res.content, kind, activeArtifact.title, vw, exportOpts),
            `${base}.pptx`,
          )
        } else if (format === "video") {
          downloadBlob(
            await exportVideo(res.content, vw, activeArtifact.viewportH, {
              scale: designConfig?.exportScale,
              onProgress,
            }),
            `${base}.mp4`,
          )
        }
        if (toastId !== undefined) toast.success(t("design.ok.exported", "已导出"), { id: toastId })
      } catch (e) {
        logger.error("design", "DesignView::handleExport", `export ${format} failed`, e)
        toast.error(t("design.err.export", "导出失败"), toastId !== undefined ? { id: toastId } : undefined)
      } finally {
        setExporting(null)
      }
    },
    [tx, activeArtifact, exporting, t, designConfig],
  )

  // 项目级 ZIP：打包该项目全部产物（每产物一目录 + 根 index.html 画廊）。
  const [exportingProject, setExportingProject] = useState(false)
  const exportProject = useCallback(async () => {
    if (!activeProject || exportingProject) return
    setExportingProject(true)
    const toastId = toast.loading(t("design.exporting", "正在导出…"))
    try {
      const res = await tx.call<{ zip: string }>("export_design_zip_cmd", {
        projectId: activeProject.id,
      })
      if (!res?.zip) return
      downloadBlob(base64ToBlob(res.zip, "application/zip"), `${safeFilename(activeProject.title)}.zip`)
      toast.success(t("design.ok.exported", "已导出"), { id: toastId })
    } catch (e) {
      logger.error("design", "DesignView::exportProject", "export project failed", e)
      toast.error(t("design.err.export", "导出失败"), { id: toastId })
    } finally {
      setExportingProject(false)
    }
  }, [tx, activeProject, exportingProject, t])

  // ── DESIGN.md 规范：导入 / 导出设计系统（互通格式）──────────────
  const [importMdOpen, setImportMdOpen] = useState(false)
  const [importMdName, setImportMdName] = useState("")
  const [importMdText, setImportMdText] = useState("")
  const [importingMd, setImportingMd] = useState(false)
  const runImportDesignMd = useCallback(async () => {
    if (!importMdText.trim()) return
    setImportingMd(true)
    try {
      const meta = await tx.call<DesignSystemMeta>("import_design_md_cmd", {
        name: importMdName.trim(),
        md: importMdText,
      })
      await loadSystems()
      if (activeProject && meta) await setProjectSystem(meta.id)
      setImportMdOpen(false)
      setImportMdText("")
      setImportMdName("")
      toast.success(t("design.ok.imported", "已导入设计系统"))
    } catch (e) {
      logger.error("design", "DesignView::importDesignMd", "import failed", e)
      toast.error(t("design.err.importMd", "DESIGN.md 导入失败"))
    } finally {
      setImportingMd(false)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tx, importMdName, importMdText, activeProject, t])
  const exportDesignMd = useCallback(
    async (systemId: string, name: string) => {
      try {
        const res = await tx.call<{ designMd: string }>("export_design_md_cmd", { systemId })
        if (!res?.designMd) return
        downloadBlob(
          new Blob([res.designMd], { type: "text/markdown" }),
          `${safeFilename(name)}-DESIGN.md`,
        )
        toast.success(t("design.ok.exported", "已导出"))
      } catch (e) {
        logger.error("design", "DesignView::exportDesignMd", "export failed", e)
        toast.error(t("design.err.export", "导出失败"))
      }
    },
    [tx, t],
  )

  // ── Version history (D1) ─────────────────────────────────────
  const [historyOpen, setHistoryOpen] = useState(false)
  const [versions, setVersions] = useState<DesignArtifactVersion[]>([])
  const [restoring, setRestoring] = useState<number | null>(null)
  const openHistory = useCallback(async () => {
    if (!activeArtifact) return
    setHistoryOpen(true)
    try {
      const list = await tx.call<DesignArtifactVersion[]>("list_design_artifact_versions_cmd", {
        id: activeArtifact.id,
      })
      setVersions(list ?? [])
    } catch (e) {
      logger.error("design", "DesignView::openHistory", "list versions failed", e)
      toast.error(t("design.err.load", "加载失败"))
    }
  }, [tx, activeArtifact, t])
  const restoreVersion = useCallback(
    async (versionId: number) => {
      if (!activeArtifact) return
      setRestoring(versionId)
      try {
        await tx.call("restore_design_version_cmd", { artifactId: activeArtifact.id, versionId })
        setPreviewKey((k) => k + 1)
        await refreshView() // sync bodyHash/currentVersion so the next visual edit isn't stale
        setHistoryOpen(false)
        if (activeProject) void loadArtifacts(activeProject.id)
        toast.success(t("design.ok.restored", "已恢复到该版本"))
      } catch (e) {
        logger.error("design", "DesignView::restoreVersion", "restore failed", e)
        toast.error(t("design.err.restore", "恢复失败"))
      } finally {
        setRestoring(null)
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [tx, activeArtifact, activeProject, refreshView, t],
  )

  // ── Reverse-extraction (D2) ──────────────────────────────────
  const [extractOpen, setExtractOpen] = useState(false)
  const [extractFrom, setExtractFrom] = useState<"brief" | "url" | "codebase" | "image">("brief")
  const [extractName, setExtractName] = useState("")
  const [extractText, setExtractText] = useState("")
  const [extracting, setExtracting] = useState(false)
  const runExtract = useCallback(async () => {
    setExtracting(true)
    try {
      const input: Record<string, unknown> = {
        name: extractName.trim() || t("design.extractedSystem", "提取的设计系统"),
        from: extractFrom,
      }
      if (extractFrom === "brief") input.brief = extractText
      else if (extractFrom === "url") input.url = extractText
      else input.path = extractText
      await tx.call("extract_design_system_cmd", { input })
      setExtractOpen(false)
      setExtractText("")
      setExtractName("")
      await loadSystems()
      toast.success(t("design.ok.extracted", "已提取设计系统"))
    } catch (e) {
      logger.error("design", "DesignView::runExtract", "extract failed", e)
      toast.error(t("design.err.extract", "反向提取失败"))
    } finally {
      setExtracting(false)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tx, extractFrom, extractName, extractText, t])

  // ── Direction picker (D2) ────────────────────────────────────
  const [directionsOpen, setDirectionsOpen] = useState(false)
  const [dirBrief, setDirBrief] = useState("")
  const [directions, setDirections] = useState<DesignDirection[]>([])
  const [proposing, setProposing] = useState(false)
  const [proposedOnce, setProposedOnce] = useState(false)
  const runProposeDirections = useCallback(async () => {
    setProposing(true)
    setProposedOnce(true)
    setDirections([])
    try {
      const list = await tx.call<DesignDirection[]>("propose_design_directions_cmd", {
        brief: dirBrief,
        count: 4,
      })
      setDirections(list ?? [])
    } catch (e) {
      logger.error("design", "DesignView::proposeDirections", "propose failed", e)
      toast.error(t("design.err.propose", "生成方向失败"))
    } finally {
      setProposing(false)
    }
  }, [tx, dirBrief, t])
  const [adopting, setAdopting] = useState<number | null>(null)
  const adoptDirection = useCallback(
    async (d: DesignDirection, index: number) => {
      setAdopting(index)
      try {
        const meta = await tx.call<DesignSystemMeta>("save_design_system_cmd", {
          input: {
            name: d.name,
            summary: d.summary,
            systemMd: `# ${d.name}\n\n${d.summary}\n`,
            tokens: d.tokens,
            source: "user",
          },
        })
        await loadSystems()
        if (activeProject && meta) await setProjectSystem(meta.id)
        setDirectionsOpen(false)
        toast.success(t("design.ok.adopted", "已应用设计方向"))
      } catch (e) {
        logger.error("design", "DesignView::adoptDirection", "adopt failed", e)
        toast.error(t("design.err.adopt", "采用方向失败"))
      } finally {
        setAdopting(null)
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [tx, activeProject, t],
  )

  // ── Quality gate (Phase 6) ───────────────────────────────────
  const [critiquing, setCritiquing] = useState(false)
  const [critique, setCritique] = useState<CritiqueResult | null>(null)
  useEffect(() => setCritique(null), [activeArtifact?.id])
  const handleCritique = useCallback(async () => {
    if (!activeArtifact) return
    setCritiquing(true)
    setCritique(null)
    try {
      const r = await tx.call<CritiqueResult>("critique_design_artifact_cmd", {
        id: activeArtifact.id,
      })
      if (r) setCritique(r)
    } catch (e) {
      logger.error("design", "DesignView::handleCritique", "critique failed", e)
      toast.error(t("design.err.critique", "质量评审失败"))
    } finally {
      setCritiquing(false)
    }
  }, [tx, activeArtifact, t])

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
      toast.error(t("design.err.delete", "删除失败"))
    } finally {
      setDeleteTarget(null)
    }
  }, [deleteTarget, tx, activeProject, activeArtifact, backToHome, loadProjects, loadArtifacts, t])

  // ── Live events ──────────────────────────────────────────────

  useEffect(() => {
    const off = [
      tx.listen("design:artifact_ready", () => {
        const proj = activeProjectRef.current
        if (proj) void loadArtifacts(proj.id)
        else void loadProjects()
      }),
      tx.listen("design:artifact_deleted", (raw) => {
        const p = parsePayload<{ artifactId?: string }>(raw)
        // Deleted artifact is the one being previewed → clear it so we don't leave a
        // broken iframe pointing at a now-removed directory.
        if (p?.artifactId && activeArtifactRef.current?.id === p.artifactId) {
          setActiveArtifact(null)
        }
        const proj = activeProjectRef.current
        if (proj) void loadArtifacts(proj.id)
      }),
      tx.listen("design:reload", (raw) => {
        const p = parsePayload<{ artifactId?: string }>(raw)
        const active = activeArtifactRef.current
        // Self-initiated visual edits already show via live preview — skip the
        // remount flash (source + oidmap are fresh; bodyHash refreshed separately).
        if (suppressReloadRef.current) {
          suppressReloadRef.current = false
        } else if (!active || !p?.artifactId || p.artifactId === active.id) {
          setPreviewKey((k) => k + 1)
          // External change (e.g. agent edit) → resync bodyHash/currentVersion so the
          // next visual edit doesn't trip the stale-write guard and get lost.
          if (active && (!p?.artifactId || p.artifactId === active.id)) void refreshView()
        }
        const proj = activeProjectRef.current
        if (proj) void loadArtifacts(proj.id)
      }),
      tx.listen("design:project_changed", () => {
        if (!activeProjectRef.current) void loadProjects()
      }),
      // Agent created / extracted a design system → refresh the picker.
      tx.listen("design:system_changed", () => {
        void loadSystems()
      }),
      // Agent ran a critique → refresh scores in the artifact list.
      tx.listen("design:critiqued", () => {
        const proj = activeProjectRef.current
        if (proj) void loadArtifacts(proj.id)
      }),
      // Agent called design(action=show): focus that artifact (auto-enter project).
      tx.listen("design:show", (raw) => {
        const p = parsePayload<{ projectId?: string; artifactId?: string }>(raw)
        if (!p?.artifactId) return
        void (async () => {
          try {
            if (p.projectId && activeProjectRef.current?.id !== p.projectId) {
              const proj = await tx.call<DesignProject | null>("get_design_project_cmd", {
                id: p.projectId,
              })
              if (proj) openProject(proj)
            }
            const artifact = await tx.call<DesignArtifact | null>("get_design_artifact_cmd", {
              id: p.artifactId,
            })
            if (artifact) void openArtifact(artifact)
          } catch (e) {
            logger.error("design", "DesignView::onShow", "focus artifact failed", e)
          }
        })()
      }),
    ]
    return () => off.forEach((f) => f())
  }, [tx, loadArtifacts, loadProjects, loadSystems, openProject, openArtifact, refreshView])

  // ── Preview iframe src ───────────────────────────────────────

  const iframeSrc = activeArtifact
    ? tx.resolveAssetUrl(`${activeArtifact.artifactPath}/index.html`) ?? ""
    : ""

  // Preview scaling. "fit" stretches the iframe to fill the pane. A numeric zoom
  // renders at the artifact's natural viewport size and visually scales it, with the
  // wrapper reserving the *scaled* footprint so 100% shows real pixels (not a no-op
  // vs. fit) and 50% shows the whole design at half size with correct scrolling.
  const naturalW = activeArtifact?.viewportW && activeArtifact.viewportW > 0 ? activeArtifact.viewportW : 1024
  const naturalH = activeArtifact?.viewportH && activeArtifact.viewportH > 0 ? activeArtifact.viewportH : 768
  const scaleStyle: CSSProperties =
    zoom === "fit"
      ? { width: "100%", height: "100%", border: 0 }
      : {
          width: `${naturalW}px`,
          height: `${naturalH}px`,
          border: 0,
          transform: `scale(${zoom})`,
          transformOrigin: "top left",
        }
  const frameWrapStyle: CSSProperties | undefined =
    zoom === "fit" ? undefined : { width: `${naturalW * zoom}px`, height: `${naturalH * zoom}px` }

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
            <>
              <Button
                variant="outline"
                size="sm"
                className="h-8 gap-1.5"
                onClick={() => setSystemPickerOpen(true)}
              >
                <Palette className="h-3.5 w-3.5 opacity-70" />
                <span className="max-w-[120px] truncate">
                  {systems.find((s) => s.id === activeProject.defaultSystemId)?.name ??
                    t("design.systemNone", "无设计系统")}
                </span>
              </Button>
              <DesignSystemPicker
                systems={systems}
                value={activeProject.defaultSystemId ?? null}
                onChange={(id) => void setProjectSystem(id)}
                open={systemPickerOpen}
                onOpenChange={setSystemPickerOpen}
                footer={
                  <div className="flex flex-wrap gap-1">
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-8 gap-1.5"
                      onClick={() => {
                        setSystemPickerOpen(false)
                        setExtractOpen(true)
                      }}
                    >
                      <Wand2 className="h-3.5 w-3.5" />
                      {t("design.extractSystem", "反向提取品牌…")}
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-8 gap-1.5"
                      onClick={() => {
                        setSystemPickerOpen(false)
                        setDirectionsOpen(true)
                      }}
                    >
                      <Sparkles className="h-3.5 w-3.5" />
                      {t("design.proposeDirections", "生成设计方向…")}
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-8 gap-1.5"
                      onClick={() => {
                        setSystemPickerOpen(false)
                        setImportMdOpen(true)
                      }}
                    >
                      <FileCode className="h-3.5 w-3.5" />
                      {t("design.importDesignMd", "导入 DESIGN.md…")}
                    </Button>
                    {activeProject.defaultSystemId && (
                      <Button
                        variant="ghost"
                        size="sm"
                        className="h-8 gap-1.5"
                        onClick={() => {
                          const sid = activeProject.defaultSystemId
                          if (!sid) return
                          const name = systems.find((s) => s.id === sid)?.name ?? sid
                          setSystemPickerOpen(false)
                          void exportDesignMd(sid, name)
                        }}
                      >
                        <FileText className="h-3.5 w-3.5" />
                        {t("design.exportDesignMd", "导出当前系统 (DESIGN.md)")}
                      </Button>
                    )}
                  </div>
                }
              />
            </>
          )}
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
                    <DropdownMenuItem key={kind} onSelect={() => onPickKind(kind)}>
                      <Icon className="mr-2 h-4 w-4" />
                      {kindLabel(kind)}
                    </DropdownMenuItem>
                  )
                })}
              </DropdownMenuContent>
            </DropdownMenu>
          )}
          {activeProject && (
            <IconTip label={t("design.exportProject", "导出项目 (ZIP)")} side="bottom">
              <Button
                variant="ghost"
                size="icon"
                className="h-8 w-8"
                disabled={exportingProject}
                onClick={() => void exportProject()}
              >
                {exportingProject ? (
                  <Loader2Icon className="h-4 w-4 animate-spin" />
                ) : (
                  <FileArchive className="h-4 w-4" />
                )}
              </Button>
            </IconTip>
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
        <LaunchHome
          projects={projects}
          loading={loadingProjects}
          systems={systems}
          prompt={homePrompt}
          setPrompt={setHomePrompt}
          kind={homeKind}
          setKind={setHomeKind}
          systemId={homeSystemId}
          setSystemId={setHomeSystemId}
          generating={generatingHome}
          onGenerate={() => void generateFromHome()}
          kindLabel={kindLabel}
          onOpen={openProject}
          onDelete={(p) => setDeleteTarget({ type: "project", id: p.id, title: p.title })}
          onNewBlank={() => setNewProjectOpen(true)}
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
                    <li key={a.id} className="group relative">
                      <button
                        type="button"
                        onClick={() => void openArtifact(a)}
                        className={cn(
                          "flex w-full items-center gap-2 rounded-lg px-2.5 py-2 pr-8 text-left text-sm transition-colors",
                          active
                            ? "bg-primary/10 text-primary"
                            : "hover:bg-muted text-foreground",
                        )}
                      >
                        <Icon className="h-4 w-4 shrink-0 opacity-70" />
                        <span className="min-w-0 flex-1 truncate">{a.title}</span>
                        {a.status === "generating" && (
                          <Loader2 className="h-3 w-3 shrink-0 animate-spin text-muted-foreground" />
                        )}
                        {a.status === "failed" && (
                          <IconTip label={t("design.statusFailed", "生成失败")} side="left">
                            <AlertCircle className="h-3.5 w-3.5 shrink-0 text-destructive" />
                          </IconTip>
                        )}
                      </button>
                      <Button
                        variant="ghost"
                        size="icon"
                        aria-label={t("common.delete", "删除")}
                        onClick={(e) => {
                          e.stopPropagation()
                          setDeleteTarget({ type: "artifact", id: a.id, title: a.title })
                        }}
                        className="absolute right-1 top-1/2 h-6 w-6 -translate-y-1/2 text-muted-foreground opacity-0 transition-opacity hover:text-destructive group-hover:opacity-100"
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </Button>
                    </li>
                  )
                })}
              </ul>
            )}
          </aside>

          {/* Single-artifact preview (center) */}
          <main className="relative flex flex-1 min-w-0 flex-col bg-muted/30">
            {activeArtifact ? (
              <>
                <div className="flex h-9 shrink-0 items-center gap-2 border-b bg-background/60 px-3">
                  <span className="truncate text-xs font-medium text-muted-foreground">
                    {activeArtifact.title}
                  </span>
                  <div className="ml-auto flex items-center gap-1">
                    {activeArtifact.kind !== "image" && (
                      <IconTip
                        label={t("design.editMode", "可视化微调：点选元素改属性")}
                        side="bottom"
                      >
                        <Button
                          variant={editMode ? "default" : "ghost"}
                          size="icon"
                          className="h-6 w-6"
                          onClick={() => setEditMode((v) => !v)}
                        >
                          <MousePointerClick className="h-3.5 w-3.5" />
                        </Button>
                      </IconTip>
                    )}
                    <Select
                      value={String(zoom)}
                      onValueChange={(v) =>
                        setZoom(v === "fit" ? "fit" : (Number(v) as ZoomMode))
                      }
                    >
                      <SelectTrigger className="h-6 w-auto gap-1 px-1.5 text-xs">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="fit">{t("design.zoomFit", "适应")}</SelectItem>
                        <SelectItem value="1">100%</SelectItem>
                        <SelectItem value="0.5">50%</SelectItem>
                      </SelectContent>
                    </Select>
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
                    {activeArtifact.kind !== "image" && (
                      <IconTip label={t("design.critique", "质量评审")} side="bottom">
                        <Button
                          variant="ghost"
                          size="icon"
                          className="h-6 w-6"
                          disabled={critiquing}
                          onClick={() => void handleCritique()}
                        >
                          {critiquing ? (
                            <Loader2Icon className="h-3.5 w-3.5 animate-spin" />
                          ) : (
                            <Gauge className="h-3.5 w-3.5" />
                          )}
                        </Button>
                      </IconTip>
                    )}
                    <IconTip label={t("design.history", "版本历史")} side="bottom">
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-6 w-6"
                        onClick={() => void openHistory()}
                      >
                        <History className="h-3.5 w-3.5" />
                      </Button>
                    </IconTip>
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button variant="ghost" size="icon" className="h-6 w-6" disabled={!!exporting}>
                          {exporting ? (
                            <Loader2Icon className="h-3.5 w-3.5 animate-spin" />
                          ) : (
                            <Download className="h-3.5 w-3.5" />
                          )}
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end">
                        <DropdownMenuItem onSelect={() => void handleExport("html")}>
                          <Code2 className="mr-2 h-4 w-4" />
                          {t("design.exportHtml", "HTML")}
                        </DropdownMenuItem>
                        <DropdownMenuItem onSelect={() => void handleExport("md")}>
                          <FileText className="mr-2 h-4 w-4" />
                          {t("design.exportMd", "Markdown")}
                        </DropdownMenuItem>
                        <DropdownMenuItem onSelect={() => void handleExport("png")}>
                          <FileImage className="mr-2 h-4 w-4" />
                          {t("design.exportPng", "PNG 图片")}
                        </DropdownMenuItem>
                        <DropdownMenuItem onSelect={() => void handleExport("pdf")}>
                          <FileText className="mr-2 h-4 w-4" />
                          {t("design.exportPdf", "PDF")}
                        </DropdownMenuItem>
                        {(activeArtifact.kind === "deck" ||
                          activeArtifact.kind === "poster" ||
                          activeArtifact.kind === "motion") && (
                          <DropdownMenuItem onSelect={() => void handleExport("pptx")}>
                            <FileType2 className="mr-2 h-4 w-4" />
                            {t("design.exportPptx", "PPTX")}
                          </DropdownMenuItem>
                        )}
                        {activeArtifact.kind === "motion" && videoExportSupported() && (
                          <DropdownMenuItem onSelect={() => void handleExport("video")}>
                            <Film className="mr-2 h-4 w-4" />
                            {t("design.exportVideo", "视频 (MP4)")}
                          </DropdownMenuItem>
                        )}
                        <DropdownMenuSeparator />
                        <DropdownMenuItem onSelect={() => void handleExport("zip")}>
                          <FileArchive className="mr-2 h-4 w-4" />
                          {t("design.exportZip", "源码包 (ZIP)")}
                        </DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </div>
                </div>
                <div className="relative flex-1 overflow-auto p-4">
                  {editMode && !selected && (
                    <div className="pointer-events-none absolute inset-x-0 top-3 z-10 flex justify-center">
                      <span className="rounded-full bg-primary/90 px-3 py-1 text-xs text-primary-foreground shadow-md">
                        {t("design.editHint", "在预览中点选一个元素开始微调")}
                      </span>
                    </div>
                  )}
                  <div
                    className={cn(
                      "overflow-hidden rounded-lg border bg-white shadow-sm",
                      zoom === "fit" ? "mx-auto h-full w-full" : "mx-auto",
                      editMode && "ring-2 ring-primary/40",
                    )}
                    style={frameWrapStyle}
                  >
                    <iframe
                      ref={iframeRef}
                      key={`${activeArtifact.id}-${previewKey}`}
                      src={iframeSrc}
                      sandbox="allow-scripts"
                      title={activeArtifact.title}
                      onLoad={handleIframeLoad}
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

            {/* Quality critique result card */}
            {critique && (
              <div className="absolute bottom-3 right-3 z-10 w-72 rounded-xl border bg-background/95 p-3 shadow-lg backdrop-blur">
                <div className="mb-2 flex items-center gap-2">
                  <Gauge className="h-4 w-4 text-primary" />
                  <span className="text-sm font-semibold">
                    {t("design.critiqueScore", "质量评分")} {critique.overall.toFixed(1)}
                  </span>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="ml-auto h-5 w-5"
                    onClick={() => setCritique(null)}
                  >
                    <X className="h-3 w-3" />
                  </Button>
                </div>
                <div className="grid grid-cols-2 gap-x-3 gap-y-0.5 text-xs">
                  {(
                    [
                      ["brand", critique.brand],
                      ["accessibility", critique.accessibility],
                      ["hierarchy", critique.hierarchy],
                      ["usability", critique.usability],
                      ["performance", critique.performance],
                    ] as const
                  ).map(([k, v]) => (
                    <div key={k} className="flex justify-between">
                      <span className="text-muted-foreground">{t(`design.dim.${k}`, k)}</span>
                      <span className="font-mono">{v.toFixed(1)}</span>
                    </div>
                  ))}
                </div>
                {critique.summary && (
                  <p className="mt-2 text-xs text-muted-foreground">{critique.summary}</p>
                )}
                {critique.fixes.length > 0 && (
                  <ul className="mt-2 list-disc space-y-0.5 pl-4 text-xs">
                    {critique.fixes.slice(0, 5).map((f, i) => (
                      <li key={i}>{f}</li>
                    ))}
                  </ul>
                )}
              </div>
            )}
          </main>

          {/* Inspector (right) — visual fine-tuning */}
          {editMode && selected && activeArtifact && (
            <DesignInspector
              selected={selected}
              onLiveStyle={handleLiveStyle}
              onCommitStyle={handleCommitStyle}
              onLiveText={handleLiveText}
              onCommitText={handleCommitText}
              onClose={() => setSelected(null)}
            />
          )}
        </div>
      )}

      {/* Image prompt dialog */}
      <Dialog open={imagePromptOpen} onOpenChange={setImagePromptOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <Sparkles className="h-4 w-4" />
              {t("design.newImage", "生成图像")}
            </DialogTitle>
          </DialogHeader>
          <Textarea
            autoFocus
            value={imagePrompt}
            onChange={(e) => setImagePrompt(e.target.value)}
            rows={3}
            placeholder={t("design.imagePromptPlaceholder", "描述你想要的图像…")}
            className="resize-none"
          />
          <DialogFooter>
            <Button variant="ghost" onClick={() => setImagePromptOpen(false)}>
              {t("common.cancel", "取消")}
            </Button>
            <Button onClick={() => void confirmImagePrompt()} disabled={creatingImage || !imagePrompt.trim()}>
              {creatingImage && <Loader2Icon className="mr-2 h-4 w-4 animate-spin" />}
              {t("design.generate", "生成")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Version history dialog */}
      <Dialog open={historyOpen} onOpenChange={setHistoryOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <History className="h-4 w-4" />
              {t("design.history", "版本历史")}
            </DialogTitle>
          </DialogHeader>
          <div className="max-h-80 space-y-1.5 overflow-y-auto">
            {versions.length === 0 ? (
              <div className="py-6 text-center text-sm text-muted-foreground">
                {t("design.noVersions", "暂无版本")}
              </div>
            ) : (
              versions.map((v) => (
                <div
                  key={v.versionNumber}
                  className="flex items-center gap-2 rounded-lg border px-3 py-2 text-sm"
                >
                  <span className="font-mono text-xs text-muted-foreground">v{v.versionNumber}</span>
                  <span className="min-w-0 flex-1 truncate">
                    {v.message ?? t("design.version", "版本")}
                  </span>
                  <span className="text-xs text-muted-foreground">
                    {new Date(v.createdAt).toLocaleString()}
                  </span>
                  {v.versionNumber !== activeArtifact?.currentVersion && (
                    <IconTip label={t("design.restore", "恢复")} side="left">
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-7 w-7 shrink-0"
                        disabled={restoring === v.versionNumber}
                        onClick={() => void restoreVersion(v.versionNumber)}
                      >
                        {restoring === v.versionNumber ? (
                          <Loader2Icon className="h-3.5 w-3.5 animate-spin" />
                        ) : (
                          <RotateCcw className="h-3.5 w-3.5" />
                        )}
                      </Button>
                    </IconTip>
                  )}
                </div>
              ))
            )}
          </div>
        </DialogContent>
      </Dialog>

      {/* Reverse-extraction dialog (D2) */}
      <Dialog open={extractOpen} onOpenChange={setExtractOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <Wand2 className="h-4 w-4" />
              {t("design.extractSystem", "反向提取品牌")}
            </DialogTitle>
          </DialogHeader>
          <div className="flex gap-1.5">
            {(["brief", "url", "image", "codebase"] as const).map((f) => (
              <Button
                key={f}
                variant={extractFrom === f ? "default" : "outline"}
                size="sm"
                className="flex-1"
                onClick={() => setExtractFrom(f)}
              >
                {t(`design.from.${f}`, f)}
              </Button>
            ))}
          </div>
          <Input
            value={extractName}
            onChange={(e) => setExtractName(e.target.value)}
            placeholder={t("design.systemNamePlaceholder", "设计系统名称")}
          />
          <Textarea
            value={extractText}
            onChange={(e) => setExtractText(e.target.value)}
            rows={4}
            placeholder={t(`design.extractHint.${extractFrom}`, "")}
            className="resize-none"
          />
          <DialogFooter>
            <Button variant="ghost" onClick={() => setExtractOpen(false)}>
              {t("common.cancel", "取消")}
            </Button>
            <Button onClick={() => void runExtract()} disabled={extracting || !extractText.trim()}>
              {extracting && <Loader2Icon className="mr-2 h-4 w-4 animate-spin" />}
              {t("design.extract", "提取")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Import DESIGN.md dialog (互通格式) */}
      <Dialog open={importMdOpen} onOpenChange={setImportMdOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <FileCode className="h-4 w-4" />
              {t("design.importDesignMd", "导入 DESIGN.md")}
            </DialogTitle>
          </DialogHeader>
          <Input
            value={importMdName}
            onChange={(e) => setImportMdName(e.target.value)}
            placeholder={t("design.systemNamePlaceholder", "设计系统名称")}
          />
          <Textarea
            value={importMdText}
            onChange={(e) => setImportMdText(e.target.value)}
            rows={10}
            placeholder={t("design.importDesignMdPlaceholder", "粘贴 DESIGN.md 文本（9 段规范 + --ds-* Token 表；缺 token 时自动合成）…")}
            className="resize-none font-mono text-xs"
          />
          <DialogFooter>
            <Button variant="ghost" onClick={() => setImportMdOpen(false)}>
              {t("common.cancel", "取消")}
            </Button>
            <Button onClick={() => void runImportDesignMd()} disabled={importingMd || !importMdText.trim()}>
              {importingMd && <Loader2Icon className="mr-2 h-4 w-4 animate-spin" />}
              {t("design.import", "导入")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Direction picker dialog (D2) */}
      <Dialog open={directionsOpen} onOpenChange={setDirectionsOpen}>
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <Sparkles className="h-4 w-4" />
              {t("design.proposeDirections", "生成设计方向")}
            </DialogTitle>
          </DialogHeader>
          <div className="flex gap-2">
            <Input
              value={dirBrief}
              onChange={(e) => setDirBrief(e.target.value)}
              placeholder={t("design.directionBriefPlaceholder", "描述你的产品 / 品牌…")}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !proposing && dirBrief.trim()) void runProposeDirections()
              }}
            />
            <Button onClick={() => void runProposeDirections()} disabled={proposing || !dirBrief.trim()}>
              {proposing && <Loader2Icon className="mr-2 h-4 w-4 animate-spin" />}
              {t("design.generate", "生成")}
            </Button>
          </div>
          {directions.length > 0 ? (
            <div className="grid grid-cols-2 gap-3">
              {directions.map((d, i) => (
                <button
                  key={i}
                  type="button"
                  disabled={adopting !== null}
                  onClick={() => void adoptDirection(d, i)}
                  className="group relative flex flex-col gap-2 rounded-xl border p-3 text-left transition-colors hover:border-primary/50 disabled:opacity-60"
                >
                  {adopting === i && (
                    <div className="absolute inset-0 z-10 flex items-center justify-center rounded-xl bg-background/60">
                      <Loader2Icon className="h-4 w-4 animate-spin text-primary" />
                    </div>
                  )}
                  <div className="flex gap-1.5">
                    {["--ds-color-primary", "--ds-color-accent", "--ds-color-bg", "--ds-color-fg"].map(
                      (k) => (
                        <span
                          key={k}
                          className="h-6 w-6 rounded-full border"
                          style={{ background: d.tokens[k] ?? "transparent" }}
                        />
                      ),
                    )}
                  </div>
                  <div className="text-sm font-medium">{d.name}</div>
                  <div className="text-xs text-muted-foreground">{d.summary}</div>
                  <div className="text-xs font-medium text-primary opacity-0 group-hover:opacity-100">
                    {t("design.useThisDirection", "采用此方向 →")}
                  </div>
                </button>
              ))}
            </div>
          ) : (
            proposedOnce &&
            !proposing && (
              <div className="py-6 text-center text-sm text-muted-foreground">
                {t("design.noDirections", "未生成方向，换个描述再试")}
              </div>
            )
          )}
        </DialogContent>
      </Dialog>

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
              {deleteTarget ? ` — ${deleteTarget.title}` : ""}
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

// ── Thumbnails ──────────────────────────────────────────────────
// 静态渲染缩略图：懒挂载（IntersectionObserver）+ sandbox=""（**不跑 JS**，画廊零动画
// 开销、性能稳定）+ ResizeObserver 等比缩放。复用产物 index.html 的 asset 服务，无需另建
// 缩略图存储管线。

const THUMB_DESIGN_W = 1280

function ArtifactThumb({ artifactId }: { artifactId: string }) {
  const wrapRef = useRef<HTMLDivElement>(null)
  const [src, setSrc] = useState<string | null>(null)
  const [scale, setScale] = useState(0.2)

  useEffect(() => {
    const el = wrapRef.current
    if (!el) return
    const ro = new ResizeObserver(() => {
      if (el.clientWidth > 0) setScale(el.clientWidth / THUMB_DESIGN_W)
    })
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  useEffect(() => {
    const el = wrapRef.current
    if (!el) return
    let done = false
    const io = new IntersectionObserver(
      (entries) => {
        if (done || !entries.some((e) => e.isIntersecting)) return
        done = true
        io.disconnect()
        getTransport()
          .call<DesignArtifactView | null>("get_design_artifact_cmd", { id: artifactId })
          .then((v) => {
            const p = v?.artifactPath
            if (p) {
              const url = getTransport().resolveAssetUrl(`${p}/index.html`)
              if (url) setSrc(url)
            }
          })
          .catch(() => {})
      },
      { rootMargin: "300px" },
    )
    io.observe(el)
    return () => io.disconnect()
  }, [artifactId])

  return (
    <div
      ref={wrapRef}
      className="relative h-full w-full overflow-hidden bg-gradient-to-br from-muted to-muted/40"
    >
      {src ? (
        <iframe
          src={src}
          sandbox=""
          scrolling="no"
          tabIndex={-1}
          aria-hidden="true"
          title=""
          className="pointer-events-none absolute left-0 top-0 origin-top-left border-0"
          style={{
            width: THUMB_DESIGN_W,
            height: THUMB_DESIGN_W * 0.75,
            transform: `scale(${scale})`,
          }}
        />
      ) : (
        <div className="flex h-full items-center justify-center">
          <Palette className="h-6 w-6 text-muted-foreground/25" />
        </div>
      )}
    </div>
  )
}

/** 项目卡缩略图：懒取该项目最近一个产物 → 渲染其静态缩略图。 */
function ProjectThumb({ projectId }: { projectId: string }) {
  const wrapRef = useRef<HTMLDivElement>(null)
  const [artifactId, setArtifactId] = useState<string | null>(null)

  useEffect(() => {
    const el = wrapRef.current
    if (!el) return
    let done = false
    const io = new IntersectionObserver(
      (entries) => {
        if (done || !entries.some((e) => e.isIntersecting)) return
        done = true
        io.disconnect()
        getTransport()
          .call<DesignArtifact[]>("list_design_artifacts_cmd", { projectId })
          .then((list) => {
            const a = list?.[0]
            if (a) setArtifactId(a.id)
          })
          .catch(() => {})
      },
      { rootMargin: "300px" },
    )
    io.observe(el)
    return () => io.disconnect()
  }, [projectId])

  return (
    <div ref={wrapRef} className="h-full w-full">
      {artifactId ? (
        <ArtifactThumb artifactId={artifactId} />
      ) : (
        <div className="flex h-full items-center justify-center bg-gradient-to-br from-muted to-muted/40">
          <Palette className="h-7 w-7 text-muted-foreground/30" />
        </div>
      )}
    </div>
  )
}

// ── Prompt-first launch home ────────────────────────────────────

function LaunchHome({
  projects,
  loading,
  systems,
  prompt,
  setPrompt,
  kind,
  setKind,
  systemId,
  setSystemId,
  generating,
  onGenerate,
  kindLabel,
  onOpen,
  onDelete,
  onNewBlank,
}: {
  projects: DesignProject[]
  loading: boolean
  systems: DesignSystemMeta[]
  prompt: string
  setPrompt: (v: string) => void
  kind: ArtifactKind
  setKind: (k: ArtifactKind) => void
  systemId: string | null
  setSystemId: (id: string | null) => void
  generating: boolean
  onGenerate: () => void
  kindLabel: (k: ArtifactKind) => string
  onOpen: (p: DesignProject) => void
  onDelete: (p: DesignProject) => void
  onNewBlank: () => void
}) {
  const { t } = useTranslation()
  const [pickerOpen, setPickerOpen] = useState(false)
  const systemName = systems.find((s) => s.id === systemId)?.name

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="mx-auto max-w-3xl px-6 pb-12 pt-14">
        {/* Hero */}
        <div className="mb-7 text-center">
          <div className="mb-4 inline-flex items-center gap-2">
            <span className="flex h-9 w-9 items-center justify-center rounded-xl bg-primary/10">
              <Palette className="h-5 w-5 text-primary" />
            </span>
            <span className="text-base font-semibold">{t("design.title", "设计空间")}</span>
          </div>
          <h1 className="font-serif text-4xl font-semibold tracking-tight sm:text-5xl">
            {t("design.launchHeading", "你想设计什么？")}
          </h1>
          <p className="mx-auto mt-3 max-w-lg text-sm text-muted-foreground">
            {t("design.launchSub", "一句话描述，直接生成可交付的设计——网页 / 演示 / 海报 / 文档 / 动效。")}
          </p>
        </div>

        {/* Prompt card */}
        <div className="rounded-2xl border bg-card p-2.5 shadow-sm transition-shadow focus-within:shadow-md focus-within:ring-2 focus-within:ring-primary/25">
          <Textarea
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
            onKeyDown={(e) => {
              if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
                e.preventDefault()
                onGenerate()
              }
            }}
            placeholder={t(
              "design.launchPlaceholder",
              "描述你想要的设计，例如「一个 SaaS 产品的定价页，三档套餐」…",
            )}
            className="min-h-[92px] resize-none border-0 bg-transparent px-2 py-1.5 text-base shadow-none focus-visible:ring-0"
          />
          <div className="mt-1 flex items-center justify-between gap-2 border-t px-1 pt-2">
            <Button
              variant="ghost"
              size="sm"
              className="h-8 gap-1.5 text-muted-foreground"
              onClick={() => setPickerOpen(true)}
            >
              <Palette className="h-3.5 w-3.5" />
              <span className="max-w-[150px] truncate">
                {systemName ?? t("design.systemNone", "无设计系统")}
              </span>
            </Button>
            <Button
              size="sm"
              className="h-9 gap-1.5"
              disabled={!prompt.trim() || generating}
              onClick={onGenerate}
            >
              {generating ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <Sparkles className="h-4 w-4" />
              )}
              {generating ? t("design.generating", "生成中…") : t("design.generate", "生成")}
            </Button>
          </div>
        </div>

        {/* Kind chips */}
        <div className="mt-4 flex flex-wrap justify-center gap-2">
          {ARTIFACT_KINDS.map((k) => {
            const Icon = KIND_ICON[k]
            const active = k === kind
            return (
              <button
                key={k}
                type="button"
                onClick={() => setKind(k)}
                className={cn(
                  "flex items-center gap-1.5 rounded-full border px-3 py-1.5 text-sm transition-colors",
                  active
                    ? "border-primary bg-primary/10 text-primary"
                    : "text-muted-foreground hover:border-primary/40 hover:text-foreground",
                )}
              >
                <Icon className="h-3.5 w-3.5" />
                {kindLabel(k)}
              </button>
            )
          })}
        </div>

        {/* Recent projects */}
        <div className="mt-12">
          <div className="mb-3 flex items-center justify-between">
            <h2 className="text-sm font-semibold text-muted-foreground">
              {t("design.recentProjects", "最近的项目")}
            </h2>
            <Button
              variant="ghost"
              size="sm"
              className="h-7 gap-1 text-xs text-muted-foreground"
              onClick={onNewBlank}
            >
              <Plus className="h-3.5 w-3.5" />
              {t("design.newBlankProject", "空白项目")}
            </Button>
          </div>

          {loading ? (
            <div className="flex justify-center py-12">
              <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
            </div>
          ) : projects.length === 0 ? (
            <div className="rounded-xl border border-dashed py-10 text-center text-sm text-muted-foreground">
              {t("design.emptyProjectsHint", "还没有项目——在上面描述一个设计，直接开始。")}
            </div>
          ) : (
            <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4">
              {projects.map((p) => (
                <div
                  key={p.id}
                  className="group relative flex flex-col overflow-hidden rounded-xl border bg-card transition-shadow hover:shadow-md"
                >
                  <button
                    type="button"
                    onClick={() => onOpen(p)}
                    disabled={generating}
                    aria-label={p.title}
                    className={cn(
                      "flex flex-1 flex-col text-left",
                      generating && "pointer-events-none opacity-60",
                    )}
                  >
                    <div
                      className="aspect-[4/3] overflow-hidden"
                      style={p.color ? { background: p.color } : undefined}
                    >
                      <ProjectThumb projectId={p.id} />
                    </div>
                    <div className="p-3 pr-9">
                      <div className="truncate text-sm font-medium">{p.title}</div>
                      <div className="text-xs text-muted-foreground">
                        {t("design.artifactCount", "{{count}} 个产物", {
                          count: p.artifactCount ?? 0,
                        })}
                      </div>
                    </div>
                  </button>
                  <Button
                    variant="ghost"
                    size="icon"
                    aria-label={t("common.delete", "删除")}
                    onClick={(e) => {
                      e.stopPropagation()
                      onDelete(p)
                    }}
                    className="absolute bottom-2 right-2 h-7 w-7 text-muted-foreground opacity-0 transition-opacity hover:text-destructive group-hover:opacity-100"
                  >
                    <Trash2 className="h-4 w-4" />
                  </Button>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>

      <DesignSystemPicker
        systems={systems}
        value={systemId}
        onChange={setSystemId}
        open={pickerOpen}
        onOpenChange={setPickerOpen}
      />
    </div>
  )
}
