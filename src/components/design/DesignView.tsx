/**
 * 设计空间独立视图（侧边栏入口）。
 *
 * 形态：首页（项目墙）↔ 工作室（产物库 + 单产物稳定预览）。
 * 刻意**不做无限画布**——多产物概览用纯 CSS grid 缩略图墙，单产物聚焦用一个
 * 稳定 iframe + CSS 缩放，从架构上规避旧版画布卡顿。见 docs/architecture/design-space.md。
 */

import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react"
import type { CSSProperties } from "react"
import { useTranslation } from "react-i18next"
import {
  ArrowLeft,
  Plus,
  Braces,
  Trash2,
  RefreshCw,
  Settings2,
  Palette,
  PanelLeft,
  PanelLeftClose,
  ShieldAlert,
  Loader2,
  Monitor,
  ChevronLeft,
  ChevronRight,
  TriangleAlert,
  RotateCcw,
  Smartphone,
  Presentation,
  LayoutDashboard,
  Image as ImageIcon,
  FileText,
  Mail,
  Brush,
  GitFork,
  Layers,
  ListChecks,
  Paintbrush,
  CheckCircle2,
  Sparkles,
  StickyNote,
  MousePointerClick,
  MessageSquare,
  MessagesSquare,
  Highlighter,
  SlidersHorizontal,
  Download,
  Gauge,
  Film,
  Music,
  Blocks,
  History,
  Search,
  LayoutGrid,
  List as ListIcon,
  MoreHorizontal,
  Pencil,
  Copy,
  Check,
  CheckSquare,
  FolderOpen,
  Tablet,
  Maximize2,
  Undo2,
  Redo2,
  Share2,
  Cloud,
  Wand2,
  FileImage,
  FileType2,
  FileArchive,
  FileCode,
  Frame,
  Link2,
  Code2,
  AlertCircle,
  X,
  Loader2 as Loader2Icon,
} from "lucide-react"
import { toast } from "sonner"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload } from "@/lib/transport"
import DesignInspector from "@/components/design/DesignInspector"
import DesignChatPanel, { type DesignChatPanelHandle } from "@/components/design/chat/DesignChatPanel"
import type { PendingFileQuote } from "@/types/chat"
import DesignCommentPanel from "@/components/design/DesignCommentPanel"
import { DesignSystemPicker } from "@/components/design/DesignSystemPicker"
import DesignKitModal from "@/components/design/DesignKitModal"
import DesignVersionHistoryModal from "@/components/design/DesignVersionHistoryModal"
import DesignDeployModal from "@/components/design/DesignDeployModal"
import DesignInpaintModal from "@/components/design/DesignInpaintModal"
import { DesignTokenEditor } from "@/components/design/DesignTokenEditor"
import { DesignTokenExport } from "@/components/design/DesignTokenExport"
import { DesignFigmaImport } from "@/components/design/DesignFigmaImport"
import { DesignCodeBinding } from "@/components/design/DesignCodeBinding"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"
import { Progress } from "@/components/ui/progress"
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
  DesignArtifactView,
  DesignProject,
  DesignSystemMeta,
  DesignRecipe,
  DesignSelectedElement,
  DesignDirection,
  DesignConfig,
  CritiqueResult,
  DesignComment,
  CommentPlacement,
} from "@/types/design"
import {
  ARTIFACT_KINDS,
  parseSelfCheck,
  parsePresenterNotes,
  parseDerivedFrom,
  parseIsRtl,
} from "@/types/design"
import {
  exportPng,
  exportPdf,
  exportPptx,
  base64ToBlob,
  safeFilename,
  rasterizeArtifactFull,
} from "@/lib/designExport"
import { presentSaveResult } from "./exportSave"
import { exportVideo } from "@/lib/designVideo"
import DesignDrawOverlay, { type DesignDrawSubmit } from "@/components/design/DesignDrawOverlay"
import { DeckSlideThumb } from "@/components/design/DeckSlideThumb"
import { ArtifactThumb } from "@/components/design/ArtifactThumb"
import DesignFilesPanel from "@/components/design/DesignFilesPanel"
import DesignSharePanel from "@/components/design/DesignSharePanel"
import { useTypewriterPlaceholder } from "./useTypewriterPlaceholder"
import { useClickOutside } from "@/hooks/useClickOutside"

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
  audio: Music,
  component: Blocks,
}

// 仅纯静态 HTML kind 支持可视化 oid 微调；image/audio 是媒体（data-uri）、component 是编译产物
// （产物≠源码），后端 render() 都不注 inspector bridge/oid，前端也不该暴露微调入口（否则 editMode
// 发 ds_activate 给无接收脚本的 iframe，「点选元素开始微调」横幅常驻点不掉）。与后端 editable 对齐。
function isEditableKind(kind: ArtifactKind): boolean {
  return kind !== "image" && kind !== "audio" && kind !== "component"
}

type ZoomMode = "fit" | number

// 预览设备视口（B4-3，源码级对标参照 PREVIEW_VIEWPORT_PRESETS）。`auto` = 沿用产物自然
// viewportW/H（默认，零回归）；其余固定逻辑宽高、居中缩放适配 + 设备框。
type PreviewDevice = "auto" | "desktop" | "tablet" | "mobile"
const DEVICE_PRESETS: Record<Exclude<PreviewDevice, "auto">, { w: number; h: number | null }> = {
  desktop: { w: 1440, h: null },
  tablet: { w: 820, h: 1180 },
  mobile: { w: 390, h: 844 },
}

/** 可视化编辑 undo/redo 的 inverse-patch 载荷 / 记录（B5）。 */
type PatchPayload = { styles?: [string, string][]; text?: string; attrs?: [string, string][] }
type EditOp = { oid: number; before: PatchPayload; after: PatchPayload }

/**
 * 本地图片 → 自包含 data-uri（B5）。fetch src（objectURL / Tauri convertFileSrc 均可 fetch）
 * → blob → canvas 降采样 + 字节预算，PNG 保留透明（logo）/ 其余 JPEG 压缩。产物须自包含故
 * 用 data-uri（与参照的项目相对路径分歧、记账本）。失败返回 null。
 */
async function imageToDataUri(src: string): Promise<string | null> {
  const blob = await (await fetch(src)).blob()
  if (!blob.type.startsWith("image/")) return null
  const needsAlpha = /png|gif|webp|svg/.test(blob.type)
  const bmp = await createImageBitmap(blob)
  const BUDGET = 700 * 1024 // data-uri 字符上限，控源码体积
  let last: string | null = null
  try {
    for (const maxEdge of [1600, 1200, 800, 512]) {
      const scale = Math.min(1, maxEdge / Math.max(bmp.width, bmp.height))
      const w = Math.max(1, Math.round(bmp.width * scale))
      const h = Math.max(1, Math.round(bmp.height * scale))
      const canvas = document.createElement("canvas")
      canvas.width = w
      canvas.height = h
      const ctx = canvas.getContext("2d")
      if (!ctx) return null
      ctx.drawImage(bmp, 0, 0, w, h)
      const candidates = needsAlpha
        ? [canvas.toDataURL("image/png")]
        : [0.85, 0.7, 0.55].map((q) => canvas.toDataURL("image/jpeg", q))
      for (const uri of candidates) {
        last = uri
        if (uri.length <= BUDGET) return uri
      }
    }
  } finally {
    bmp.close?.()
  }
  return last // 尽力而为：仍超预算也返回最小的一版
}

/** 涂画命中的元素成员（`ds_draw_hit` 桥回传）：oid + tag + 文本片段，带给模型做 edit_element。 */
interface DrawMember {
  oid: number
  tag: string
  snippet: string
}

/** 把 oid 元素的当前 computedStyle 压成一行紧凑提示（带给模型省一次 get_artifact）。空则 ""。 */
function formatStyleLine(styles: Record<string, string> | undefined): string {
  if (!styles) return ""
  const entries = Object.entries(styles).filter(([, v]) => v && v.trim())
  if (entries.length === 0) return ""
  return "\n当前样式：" + entries.map(([k, v]) => `${k}=${v}`).join("; ")
}

/** iframe 视口/滚动度量（B4-1，经 `ds_viewport` 桥回传；父层跨源无法直接读）。 */
interface ViewportMetrics {
  scrollX: number
  scrollY: number
  clientWidth: number
  clientHeight: number
  scrollWidth: number
  scrollHeight: number
}

/**
 * 把归一化画框批注合成到离屏整页渲染上并裁剪成聚焦 PNG（B4-1）。
 * 坐标：归一化(视口 0..1) → 产物 CSS px `ax=scrollX+nx*clientWidth` → 画布 px `ax*renderScale`
 *（`bg` 由 rasterizeArtifactFull 按 `clientWidth` 视口、`renderScale` 倍率整页渲染，故此式 1:1 对齐）。
 * 裁剪到 marks 并union bbox + 15% padding，输出封顶 1600px 长边控 token 预算。无 marks 返回 null。
 */
async function compositeAnnotation(
  bg: HTMLCanvasElement,
  renderScale: number,
  vp: ViewportMetrics,
  payload: DesignDrawSubmit,
): Promise<File | null> {
  const ctx = bg.getContext("2d")
  if (!ctx) return null
  const W = bg.width
  const H = bg.height
  const toPx = (nx: number, ny: number): [number, number] => [
    (vp.scrollX + nx * vp.clientWidth) * renderScale,
    (vp.scrollY + ny * vp.clientHeight) * renderScale,
  ]
  const STROKE = "#ff3b30"
  ctx.lineJoin = "round"
  ctx.lineCap = "round"
  const bboxes: [number, number, number, number][] = []
  for (const b of payload.boxes) {
    const [x0, y0] = toPx(b.x, b.y)
    const [x1, y1] = toPx(b.x + b.width, b.y + b.height)
    const x = Math.min(x0, x1)
    const y = Math.min(y0, y1)
    const w = Math.abs(x1 - x0)
    const h = Math.abs(y1 - y0)
    ctx.fillStyle = "rgba(255,59,48,0.10)"
    ctx.fillRect(x, y, w, h)
    ctx.strokeStyle = STROKE
    ctx.lineWidth = Math.max(2, 2 * renderScale)
    ctx.setLineDash([10 * renderScale, 6 * renderScale])
    ctx.strokeRect(x, y, w, h)
    ctx.setLineDash([])
    bboxes.push([x, y, w, h])
  }
  ctx.strokeStyle = STROKE
  ctx.lineWidth = Math.max(2, 3 * renderScale)
  for (const pts of payload.strokes) {
    if (pts.length < 2) continue
    let minx = Infinity
    let miny = Infinity
    let maxx = -Infinity
    let maxy = -Infinity
    ctx.beginPath()
    pts.forEach((p, i) => {
      const [px, py] = toPx(p.x, p.y)
      if (i === 0) ctx.moveTo(px, py)
      else ctx.lineTo(px, py)
      minx = Math.min(minx, px)
      miny = Math.min(miny, py)
      maxx = Math.max(maxx, px)
      maxy = Math.max(maxy, py)
    })
    ctx.stroke()
    bboxes.push([minx, miny, maxx - minx, maxy - miny])
  }
  if (bboxes.length === 0) return null
  let minx = Infinity
  let miny = Infinity
  let maxx = -Infinity
  let maxy = -Infinity
  for (const [x, y, w, h] of bboxes) {
    minx = Math.min(minx, x)
    miny = Math.min(miny, y)
    maxx = Math.max(maxx, x + w)
    maxy = Math.max(maxy, y + h)
  }
  const padX = Math.max(24, (maxx - minx) * 0.15)
  const padY = Math.max(24, (maxy - miny) * 0.15)
  const cx = Math.max(0, Math.floor(minx - padX))
  const cy = Math.max(0, Math.floor(miny - padY))
  const cw = Math.min(W - cx, Math.ceil(maxx - minx + padX * 2))
  const ch = Math.min(H - cy, Math.ceil(maxy - miny + padY * 2))
  if (cw <= 0 || ch <= 0) return null
  const MAX_EDGE = 1600
  const outScale = Math.min(1, MAX_EDGE / Math.max(cw, ch))
  const ow = Math.max(1, Math.round(cw * outScale))
  const oh = Math.max(1, Math.round(ch * outScale))
  const out = document.createElement("canvas")
  out.width = ow
  out.height = oh
  const octx = out.getContext("2d")
  if (!octx) return null
  octx.drawImage(bg, cx, cy, cw, ch, 0, 0, ow, oh)
  const blob: Blob | null = await new Promise((r) => out.toBlob((b) => r(b), "image/png"))
  if (!blob) return null
  return new File([blob], "design-annotation.png", { type: "image/png" })
}

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
  const [artifactsError, setArtifactsError] = useState(false) // Wave 2-⑨：库加载失败显式态

  const [newProjectOpen, setNewProjectOpen] = useState(false)
  const [newProjectTitle, setNewProjectTitle] = useState("")
  const [creatingProject, setCreatingProject] = useState(false)
  const [systemPickerOpen, setSystemPickerOpen] = useState(false)
  const [tokenEditorOpen, setTokenEditorOpen] = useState(false)
  const [tokenEditorSystem, setTokenEditorSystem] = useState<DesignSystemMeta | null>(null)
  const [tokenExportOpen, setTokenExportOpen] = useState(false)
  const [tokenExportSystem, setTokenExportSystem] = useState<DesignSystemMeta | null>(null)
  const [figmaImportOpen, setFigmaImportOpen] = useState(false)
  const [codeBindOpen, setCodeBindOpen] = useState(false)
  const [codeBindSystem, setCodeBindSystem] = useState<DesignSystemMeta | null>(null)

  const [deleteTarget, setDeleteTarget] = useState<
    | { type: "project"; id: string; title: string }
    | { type: "artifact"; id: string; title: string }
    | { type: "artifacts-batch"; ids: string[]; title: string }
    | null
  >(null)
  // 页面组织（本轮）：产物总览网格 / 就地改名（产物 + 项目）/ 拖动排序。
  const [showGrid, setShowGrid] = useState(false)
  const [folders, setFolders] = useState<string[]>([]) // 页面分组文件夹路径
  const [renamingArtifactId, setRenamingArtifactId] = useState<string | null>(null)
  const [renameDraft, setRenameDraft] = useState("")
  const [renamingProject, setRenamingProject] = useState(false)

  const [zoom, setZoom] = useState<ZoomMode>("fit")
  const [previewKey, setPreviewKey] = useState(0)
  const iframeRef = useRef<HTMLIFrameElement>(null)
  // 预览重载中（Wave 2-⑥）：src 变→true，onLoad→false；驱动叠层 spinner，让改稿读作「更新中」
  // 而非白屏/坏页。旧帧因 iframe 不再按 key 重挂而垫在下面直到新帧就绪。
  const [previewLoading, setPreviewLoading] = useState(false)
  const previewLoadingRef = useRef(false)
  previewLoadingRef.current = previewLoading
  // 各产物最近滚动位置（桥经 ds_scroll 上报），重载 onLoad 后回写实现滚动保温。
  const previewScrollRef = useRef<Map<string, { x: number; y: number }>>(new Map())
  // Deck 演示导航（Wave 2-⑧）：当前 slide 状态（桥 ds_slide_state 上报）+ 演示态独立 iframe ref。
  const presentIframeRef = useRef<HTMLIFrameElement>(null)
  const [deckState, setDeckState] = useState<{ active: number; count: number } | null>(null)
  const deckStateRef = useRef(deckState)
  deckStateRef.current = deckState
  // 预览设备视口（B4-3）+ 演示态（B4-4）。
  const [previewDevice, setPreviewDevice] = useState<PreviewDevice>("auto")
  const [presentMode, setPresentMode] = useState(false) // 本标签无 chrome 演示
  // 演讲者备注（deck，按 slide 顺序，存产物 metadata.presenterNotes）+ 演示计时器 + 备注面板开关。
  const [presenterNotes, setPresenterNotes] = useState<string[]>([])
  const [presenterOpen, setPresenterOpen] = useState(true)
  const [presentElapsed, setPresentElapsed] = useState(0)
  const previewPaneRef = useRef<HTMLDivElement>(null)
  const [paneSize, setPaneSize] = useState({ w: 0, h: 0 })

  // 设计系统套件（Kit）预览模态：选择器行内「预览套件」触发（B1-1）。
  const [kitSystem, setKitSystem] = useState<{ id: string; name: string } | null>(null)

  // AI 对话左栏（chat-to-edit：左对话 / 右预览，可拖宽 · 可折叠）。宽度持久化。
  const chatPanelRef = useRef<DesignChatPanelHandle>(null)
  const [chatOpen, setChatOpen] = useState(true)
  // 带 quote 到对话（B4 review 修复）：面板折叠时 chatPanelRef 为 null，直接 addQuote 会丢。
  // 打开面板 + 缓冲 quote，待面板挂载后经 chatOpen 副作用 flush（恰好一次）。
  const pendingQuotesRef = useRef<PendingFileQuote[]>([])
  const enqueueChatQuote = useCallback((quote: PendingFileQuote) => {
    setChatOpen(true)
    if (chatPanelRef.current) chatPanelRef.current.addQuote(quote)
    else pendingQuotesRef.current.push(quote)
  }, [])
  useEffect(() => {
    if (!chatOpen || !chatPanelRef.current || pendingQuotesRef.current.length === 0) return
    const queued = pendingQuotesRef.current
    pendingQuotesRef.current = []
    for (const q of queued) chatPanelRef.current.addQuote(q)
  }, [chatOpen])
  // 画框批注合成图作对话图附件（同 quote 缓冲：面板未挂载先缓冲、chatOpen 后 flush 恰好一次）。
  const pendingImagesRef = useRef<File[]>([])
  const enqueueChatImage = useCallback((file: File) => {
    setChatOpen(true)
    if (chatPanelRef.current) chatPanelRef.current.addImageAttachment(file)
    else pendingImagesRef.current.push(file)
  }, [])
  useEffect(() => {
    if (!chatOpen || !chatPanelRef.current || pendingImagesRef.current.length === 0) return
    const queued = pendingImagesRef.current
    pendingImagesRef.current = []
    for (const f of queued) chatPanelRef.current.addImageAttachment(f)
  }, [chatOpen])
  // 「让 AI 修复」缓冲（同 quote/image：面板折叠时 ref 为 null，先打开面板缓冲，挂载后 flush 发送）。
  const pendingFixRef = useRef<string | null>(null)
  useEffect(() => {
    if (!chatOpen || !chatPanelRef.current || !pendingFixRef.current) return
    const prompt = pendingFixRef.current
    pendingFixRef.current = null
    chatPanelRef.current.submitPrompt(prompt)
  }, [chatOpen])
  const [chatWidth, setChatWidth] = useState(() => {
    const saved = Number(localStorage.getItem("design_chat_width"))
    return Number.isFinite(saved) && saved >= 320 && saved <= 640 ? saved : 400
  })
  useEffect(() => {
    localStorage.setItem("design_chat_width", String(chatWidth))
  }, [chatWidth])
  const startChatResize = useCallback(
    (e: React.PointerEvent) => {
      e.preventDefault()
      const startX = e.clientX
      const startW = chatWidth
      const onMove = (ev: PointerEvent) => {
        setChatWidth(Math.max(320, Math.min(640, startW + ev.clientX - startX)))
      }
      const onUp = () => {
        window.removeEventListener("pointermove", onMove)
        window.removeEventListener("pointerup", onUp)
      }
      window.addEventListener("pointermove", onMove)
      window.addEventListener("pointerup", onUp)
    },
    [chatWidth],
  )

  // 可视化微调（D1）
  const [editMode, setEditMode] = useState(false)
  const [selected, setSelected] = useState<DesignSelectedElement | null>(null)
  const selectedRef = useRef<DesignSelectedElement | null>(null)
  selectedRef.current = selected
  const editModeRef = useRef(false)
  editModeRef.current = editMode
  // 编辑态预览右键菜单：bridge ds_context_menu 回传 iframe 内坐标，父层按当前预览缩放换算成
  // 窗口坐标弹菜单（非编辑态 bridge 零拦截，原生右键不受影响）。previewScaleRef 在缩放计算处赋值。
  const [previewCtxMenu, setPreviewCtxMenu] = useState<{ x: number; y: number } | null>(null)
  const previewScaleRef = useRef(1)
  // 批注钉：模式 / 数据 / 待填新钉锚点。与 editMode 互斥（都用 bridge + 右面板）。
  const [commentMode, setCommentMode] = useState(false)
  const commentModeRef = useRef(false)
  commentModeRef.current = commentMode
  const [comments, setComments] = useState<DesignComment[]>([])
  const commentsRef = useRef<DesignComment[]>([])
  commentsRef.current = comments
  const [pendingPlacement, setPendingPlacement] = useState<CommentPlacement | null>(null)
  // 点预览钉时要在面板里聚焦/编辑的批注 id（B0-3）；面板消费后回调清空。
  const [focusCommentId, setFocusCommentId] = useState<number | null>(null)
  // 画框批注（B4-1）：父层 canvas 叠层，与 editMode/commentMode 三态互斥；drawBusy=捕获/合成在途。
  const [drawMode, setDrawMode] = useState(false)
  const drawModeRef = useRef(false)
  drawModeRef.current = drawMode
  const [drawBusy, setDrawBusy] = useState(false)
  // Live refs so the EventBus subscription can read current project/artifact without
  // being a dependency (avoids re-subscribing — and dropping events — on every edit).
  const activeProjectRef = useRef<DesignProject | null>(null)
  activeProjectRef.current = activeProject
  const activeArtifactRef = useRef<DesignArtifactView | null>(null)
  activeArtifactRef.current = activeArtifact

  // 提前声明（commit handlers 在历史块之前引用；实体在 undo/redo 块内赋值）。
  const pushHistoryRef = useRef<(op: EditOp) => void>(() => {})
  const activeArtifactId = activeArtifact?.id
  const postToIframe = useCallback((msg: Record<string, unknown>) => {
    iframeRef.current?.contentWindow?.postMessage(msg, "*")
  }, [])

  // Deck 翻页指令 → 当前有效 iframe（演示态发演示 iframe，否则预览 iframe）（Wave 2-⑧）。
  const presentModeRef = useRef(false)
  presentModeRef.current = presentMode
  const deckNav = useCallback((type: string, index?: number) => {
    const win = (presentModeRef.current ? presentIframeRef.current : iframeRef.current)?.contentWindow
    win?.postMessage(index != null ? { type, index } : { type }, "*")
  }, [])
  // Deck 宿主级键盘翻页（Wave 2-⑧）：无需先点 iframe 拿焦点。编辑/批注/画框态不劫持方向键；
  // 跳过 input/textarea/contenteditable 焦点；带修饰键放行（避免抢 Cmd+Z 等）。演示态也生效。
  useEffect(() => {
    if (activeArtifact?.kind !== "deck") return
    if (!presentMode && (editMode || commentMode || drawMode)) return
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement | null
      if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable)) return
      if (e.metaKey || e.ctrlKey || e.altKey) return
      // 收窄劫持范围（review LOW）：演示态全局生效；预览态仅当焦点在预览区内或无特定焦点时，
      // 避免抢走侧栏 / 对话面板等无关 UI 的方向键。
      if (!presentMode) {
        const a = document.activeElement
        if (a && a !== document.body && !previewPaneRef.current?.contains(a)) return
      }
      if (e.key === "ArrowRight" || e.key === "PageDown") {
        e.preventDefault()
        deckNav("ds_slide_next")
      } else if (e.key === "ArrowLeft" || e.key === "PageUp") {
        e.preventDefault()
        deckNav("ds_slide_prev")
      } else if (e.key === "Home") {
        e.preventDefault()
        deckNav("ds_slide_go", 0)
      } else if (e.key === "End" && deckStateRef.current) {
        e.preventDefault()
        deckNav("ds_slide_go", deckStateRef.current.count - 1)
      }
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [activeArtifact?.kind, presentMode, editMode, commentMode, drawMode, deckNav])

  // 退出演示 → 预览 iframe 同步到演示时停留的那页（Wave 2-⑧ review MED）：否则宿主页码/翻页
  // 边界与实际预览页脱节（演示里翻到第 7 页，退出后预览仍停在进入前那页）。
  const wasPresentRef = useRef(false)
  useEffect(() => {
    if (wasPresentRef.current && !presentMode) {
      const s = deckStateRef.current
      if (activeArtifactRef.current?.kind === "deck" && s) {
        requestAnimationFrame(() =>
          iframeRef.current?.contentWindow?.postMessage(
            { type: "ds_slide_go", index: s.active },
            "*",
          ),
        )
      }
    }
    wasPresentRef.current = presentMode
  }, [presentMode])

  // ── 画框批注 orchestration（B4-1）──
  // ds_viewport round-trip：跨源无法直接读 iframe 滚动/视口，postMessage 请求 → 回传 resolve。
  const viewportReqRef = useRef(new Map<number, (m: ViewportMetrics) => void>())
  const viewportReqIdRef = useRef(0)
  const requestViewportMetrics = useCallback((): Promise<ViewportMetrics | null> => {
    const win = iframeRef.current?.contentWindow
    if (!win) return Promise.resolve(null)
    const id = ++viewportReqIdRef.current
    return new Promise((resolve) => {
      const timer = window.setTimeout(() => {
        viewportReqRef.current.delete(id)
        resolve(null)
      }, 1500)
      viewportReqRef.current.set(id, (m) => {
        window.clearTimeout(timer)
        viewportReqRef.current.delete(id)
        resolve(m)
      })
      win.postMessage({ type: "ds_viewport", id }, "*")
    })
  }, [])
  const forwardScrollToIframe = useCallback(
    (dx: number, dy: number) => postToIframe({ type: "ds_scroll_by", dx, dy }),
    [postToIframe],
  )
  // 涂画+元素身份合一：把归一化绘制区域映射到 iframe **内容坐标系**（scrollX+n*clientWidth，与
  // compositeAnnotation 同口径，不含 renderScale），请 bridge 回传被覆盖的 oid 成员 → 带给模型做
  // edit_element 精改。跨源经 postMessage round-trip（镜像 requestViewportMetrics）。
  const drawHitReqRef = useRef(new Map<number, (m: DrawMember[]) => void>())
  const drawHitIdRef = useRef(0)
  const requestDrawHits = useCallback(
    (vp: ViewportMetrics, payload: DesignDrawSubmit): Promise<DrawMember[]> => {
      const win = iframeRef.current?.contentWindow
      const cw = vp.clientWidth
      const ch = vp.clientHeight
      if (!win || cw <= 0 || ch <= 0) return Promise.resolve([])
      const regions: { x: number; y: number; w: number; h: number }[] = []
      for (const b of payload.boxes)
        regions.push({ x: vp.scrollX + b.x * cw, y: vp.scrollY + b.y * ch, w: b.width * cw, h: b.height * ch })
      for (const pts of payload.strokes) {
        if (pts.length < 1) continue
        let minx = Infinity, miny = Infinity, maxx = -Infinity, maxy = -Infinity
        for (const p of pts) {
          minx = Math.min(minx, p.x); miny = Math.min(miny, p.y)
          maxx = Math.max(maxx, p.x); maxy = Math.max(maxy, p.y)
        }
        regions.push({ x: vp.scrollX + minx * cw, y: vp.scrollY + miny * ch, w: (maxx - minx) * cw, h: (maxy - miny) * ch })
      }
      if (regions.length === 0) return Promise.resolve([])
      const id = ++drawHitIdRef.current
      return new Promise((resolve) => {
        const timer = window.setTimeout(() => {
          drawHitReqRef.current.delete(id)
          resolve([])
        }, 1500)
        drawHitReqRef.current.set(id, (m) => {
          window.clearTimeout(timer)
          drawHitReqRef.current.delete(id)
          resolve(m)
        })
        win.postMessage({ type: "ds_draw_hit", id, regions }, "*")
      })
    },
    [],
  )
  // 钉/批注带到对话时按 oid 取当前 computedStyle（省模型一次 get_artifact；富化 scope）。
  const styleReqRef = useRef(new Map<number, (m: Record<string, Record<string, string>>) => void>())
  const styleReqIdRef = useRef(0)
  const requestElementStyles = useCallback(
    (oids: number[]): Promise<Record<string, Record<string, string>>> => {
      const win = iframeRef.current?.contentWindow
      if (!win || oids.length === 0) return Promise.resolve({})
      const id = ++styleReqIdRef.current
      return new Promise((resolve) => {
        const timer = window.setTimeout(() => {
          styleReqRef.current.delete(id)
          resolve({})
        }, 1500)
        styleReqRef.current.set(id, (m) => {
          window.clearTimeout(timer)
          styleReqRef.current.delete(id)
          resolve(m)
        })
        win.postMessage({ type: "ds_style_query", id, oids }, "*")
      })
    },
    [],
  )
  const describeMarks = useCallback(
    (payload: DesignDrawSubmit, hasImage: boolean, title: string, members: DrawMember[]): string => {
      const lines: string[] = [
        t("design.draw.scopeHeader", "【画框批注】用户在产物「{{title}}」的预览上标注了要修改的区域。", {
          title,
        }),
      ]
      if (hasImage) lines.push(t("design.draw.scopeImage", "随附截图中的红框 / 红线即标注区域。"))
      else {
        const n = payload.boxes.length + payload.strokes.length
        lines.push(t("design.draw.scopeNoImage", "共 {{n}} 处标注（截图未生成，仅文字说明）。", { n }))
      }
      if (payload.note) lines.push(t("design.draw.scopeNote", "用户说明：{{note}}", { note: payload.note }))
      // 涂画+元素身份合一：命中元素带上 oid，让模型对每个用 edit_element(oid) 就地精改（保留其它一切），
      // 而非靠位图/区域描述猜元素、改错相邻元素。
      if (members.length) {
        const list = members
          .map((m) => `<${m.tag}>（oid=${m.oid}）${m.snippet ? "：" + m.snippet.slice(0, 30) : ""}`)
          .join("、")
        lines.push(t("design.draw.scopeElements", "标注命中元素：{{list}}。", { list }))
        lines.push(
          t(
            "design.draw.scopeEdit",
            "逐个用 design 工具 edit_element(oid, style/text/...) 就地精改，不确定当前样式先 get_artifact 读 source；别重造整个产物。",
          ),
        )
      }
      lines.push(t("design.draw.scopeInstruction", "请只针对标注区域修改，其余部分保持不变。"))
      return lines.join("\n")
    },
    [t],
  )
  // 提交：捕获底图（离屏整页栅格化，跨源/无 Chrome 通用）→ 合成红框/红线 → 裁剪 → 图附件 + 区域
  // 描述 quote 一起带到对话（draft 语义：用户审后手动发）。捕获失败静默降级为「区域+文字」，永不阻塞。
  const handleDrawSubmit = useCallback(
    async (payload: DesignDrawSubmit) => {
      const art = activeArtifactRef.current
      if (!art) return
      setDrawBusy(true)
      try {
        let file: File | null = null
        const vp = await requestViewportMetrics()
        const hasMarks = payload.boxes.length > 0 || payload.strokes.length > 0
        // 涂画命中的 oid 成员（跨源 round-trip；bridge 对非 oid 内容回空、安全降级为纯区域描述）。
        const members = vp && hasMarks ? await requestDrawHits(vp, payload) : []
        // deck / motion 是**多帧/多态**产物：离屏 fresh render 只会渲默认态（deck slide 1 /
        // motion 首帧），与用户所看的当前帧不符 → 底图会误导（review MED）。这类只发文字标注、
        // 不烧底图（describeMarks 的 !file 分支给「仅文字说明」），宁缺勿错。
        const captureable = art.kind !== "deck" && art.kind !== "motion"
        if (captureable && vp && vp.clientWidth > 0 && vp.clientHeight > 0 && hasMarks) {
          try {
            const res = await tx.call<{ content: string }>("export_design_artifact_cmd", {
              id: art.id,
              format: "html",
            })
            if (res?.content) {
              const { canvas, scale } = await rasterizeArtifactFull(res.content, vp.clientWidth, {
                scale: 2,
              })
              file = await compositeAnnotation(canvas, scale, vp, payload)
            }
          } catch (e) {
            logger.warn(
              "design",
              "DesignView::handleDrawSubmit",
              "capture/composite failed; degrading to text-only",
              e,
            )
          }
        }
        // 截图降级告知（此前失败只 log，用户不知截图没成）：仅在**尝试过**捕获（可截 kind）却
        // 失败时提示；deck/motion 本就刻意不截图（多帧误导），不提示。
        if (captureable && hasMarks && vp && !file) {
          toast.info(t("design.draw.captureFailed", "截图未生成，已仅发送区域文字说明"))
        }
        enqueueChatQuote({
          path: `design-draw:${art.id}:${viewportReqIdRef.current}`,
          name: t("design.draw.quoteName", "画框批注"),
          startLine: 0,
          endLine: 0,
          content: describeMarks(payload, !!file, art.title, members),
        })
        if (file) enqueueChatImage(file)
        setDrawMode(false)
      } finally {
        setDrawBusy(false)
      }
    },
    [tx, t, requestViewportMetrics, requestDrawHits, describeMarks, enqueueChatQuote, enqueueChatImage],
  )

  // 流式生成态：streamRef 追当前流（streamId 变化=新流重置、seq 丢乱序帧）；
  // snapshotRef 存最新 css/body 供 iframe 加载完 `ds_stream_ready` 时补投。
  const streamRef = useRef<{ artifactId: string; streamId: string; seq: number } | null>(null)
  const streamSnapshotRef = useRef<{ artifactId: string; css: string; bodyHtml: string } | null>(
    null,
  )

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

  // 设计模板目录（首屏模板快选）。
  const [recipes, setRecipes] = useState<DesignRecipe[]>([])
  useEffect(() => {
    getTransport()
      .call<DesignRecipe[]>("list_design_recipes_cmd")
      .then((list) => setRecipes(list ?? []))
      .catch(() => {})
  }, [])

  // Export clarity/quality prefs (config-driven; undefined → export defaults).
  const [designConfig, setDesignConfig] = useState<DesignConfig | null>(null)
  useEffect(() => {
    tx.call<DesignConfig>("get_design_config_cmd")
      .then(setDesignConfig)
      .catch(() => {})
  }, [tx])

  // 设为新对话/新项目默认设计系统（B1-3）：写 design.default_system_id；解析链 explicit >
  // 项目 default > **此全局 default** 已在后端就绪，LaunchHome 生成也已 seed 此值。
  const setDefaultSystem = useCallback(
    async (systemId: string | null) => {
      if (!designConfig) return
      const next: DesignConfig = { ...designConfig, defaultSystemId: systemId ?? undefined }
      setDesignConfig(next) // 乐观更新
      try {
        await tx.call("save_design_config_cmd", { config: next })
        toast.success(
          systemId
            ? t("design.setDefaultDone", "已设为新对话默认设计系统")
            : t("design.clearDefaultDone", "已清除默认设计系统"),
        )
      } catch (e) {
        logger.error("design", "DesignView::setDefault", "save default system failed", e)
        toast.error(t("design.err.save", "保存失败"))
      }
    },
    [tx, designConfig, t],
  )

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

  // 就地换设计系统：对当前打开的产物 restyle（后端重渲染 + 落新版本，源码不变）。
  const restyleActiveArtifact = useCallback(
    async (systemId: string | null) => {
      if (!activeArtifactRef.current) return
      try {
        await tx.call<DesignArtifact>("restyle_design_artifact_cmd", {
          id: activeArtifactRef.current.id,
          systemId: systemId ?? undefined,
        })
        await refreshView()
        setPreviewKey((k) => k + 1)
        toast.success(t("design.ok.restyled", "已换设计系统"))
      } catch (e) {
        logger.error("design", "DesignView::restyle", "restyle failed", e)
        toast.error(t("design.err.restyle", "换设计系统失败"))
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [tx, t],
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

  // 改名（复用 update_design_project_cmd 的 title 更新；空 / 未变 no-op）。
  const renameProject = useCallback(
    async (id: string, title: string) => {
      const next = title.trim()
      if (!next) return
      try {
        await tx.call<DesignProject>("update_design_project_cmd", { input: { id, title: next } })
        // 就地改名后同步当前打开项目（工作室标题读 activeProject）+ 刷新项目墙列表。
        setActiveProject((prev) => (prev && prev.id === id ? { ...prev, title: next } : prev))
        await loadProjects()
      } catch (e) {
        logger.error("design", "DesignView::renameProject", "rename failed", e)
        toast.error(t("design.err.save", "保存失败"))
      }
    },
    [tx, t, loadProjects],
  )

  // 复制项目（后端深拷贝产物 + 版本快照 + 溯源）。
  const duplicateProject = useCallback(
    async (id: string) => {
      try {
        await tx.call<DesignProject>("duplicate_design_project_cmd", { id })
        await loadProjects()
        toast.success(t("design.ok.duplicated", "已复制项目"))
      } catch (e) {
        logger.error("design", "DesignView::duplicateProject", "duplicate failed", e)
        toast.error(t("design.err.duplicate", "复制失败"))
      }
    },
    [tx, t, loadProjects],
  )

  // ── Artifacts ────────────────────────────────────────────────

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

  const loadArtifacts = useCallback(
    // `selectFirst`：打开项目时自动选中列表首项（后端按 position ASC 返回 = 第一个页面），
    // 其余调用方（新建 / 刷新后重载）不传，保留当前选中不被顶掉。
    async (projectId: string, selectFirst = false) => {
      setLoadingArtifacts(true)
      setArtifactsError(false)
      try {
        const list = await tx.call<DesignArtifact[]>("list_design_artifacts_cmd", {
          projectId,
        })
        setArtifacts(list ?? [])
        if (selectFirst && list && list.length > 0) void openArtifact(list[0])
      } catch (e) {
        // Wave 2-⑨：置显式 error 态，让产物墙显示「加载失败 + 重试」而非误当空库。
        logger.error("design", "DesignView::loadArtifacts", "list artifacts failed", e)
        setArtifactsError(true)
        toast.error(t("design.err.load", "加载失败"))
      } finally {
        setLoadingArtifacts(false)
      }
    },
    [tx, t, openArtifact],
  )

  // ── 产物（页面）改名 / 复制 / 拖动排序（本轮）──
  const renameArtifact = useCallback(
    async (id: string, title: string) => {
      const next = title.trim()
      if (!next) return
      try {
        await tx.call("rename_design_artifact_cmd", { id, title: next })
        const pid = activeProjectRef.current?.id
        if (pid) await loadArtifacts(pid)
        setActiveArtifact((prev) => (prev && prev.id === id ? { ...prev, title: next } : prev))
      } catch (e) {
        logger.error("design", "DesignView::renameArtifact", "rename failed", e)
        toast.error(t("design.err.save", "保存失败"))
      }
    },
    [tx, t, loadArtifacts],
  )
  const duplicateArtifact = useCallback(
    async (id: string) => {
      try {
        const dup = await tx.call<DesignArtifact>("duplicate_design_artifact_cmd", { id })
        const pid = activeProjectRef.current?.id
        if (pid) await loadArtifacts(pid)
        if (dup) void openArtifact(dup)
      } catch (e) {
        logger.error("design", "DesignView::duplicateArtifact", "duplicate failed", e)
        toast.error(t("design.err.save", "保存失败"))
      }
    },
    [tx, t, loadArtifacts, openArtifact],
  )
  const reorderArtifacts = useCallback(
    async (orderedIds: string[]) => {
      const pid = activeProjectRef.current?.id
      if (!pid) return
      // 乐观更新：立即按新顺序重排本地 artifacts，拖拽结果即时反映（review MED），
      // 失败再 loadArtifacts 回滚到服务器真相。
      setArtifacts((prev) => {
        const rank = new Map(orderedIds.map((id, i) => [id, i]))
        return [...prev].sort((a, b) => {
          const ra = rank.get(a.id)
          const rb = rank.get(b.id)
          if (ra == null && rb == null) return 0
          if (ra == null) return 1
          if (rb == null) return -1
          return ra - rb
        })
      })
      try {
        await tx.call("reorder_design_artifacts_cmd", { projectId: pid, orderedIds })
      } catch (e) {
        logger.error("design", "DesignView::reorderArtifacts", "reorder failed", e)
        await loadArtifacts(pid) // 回滚到服务器真相
      }
    },
    [tx, loadArtifacts],
  )
  // ── 页面分组文件夹（本轮·复刻 OD）──
  const loadFolders = useCallback(
    async (projectId: string) => {
      try {
        const list = await tx.call<string[]>("list_design_folders_cmd", { projectId })
        setFolders(list ?? [])
      } catch (e) {
        logger.error("design", "DesignView::loadFolders", "list folders failed", e)
      }
    },
    [tx],
  )
  const createFolder = useCallback(
    async (path: string) => {
      const pid = activeProjectRef.current?.id
      if (!pid) return
      try {
        await tx.call("create_design_folder_cmd", { projectId: pid, name: path })
        await loadFolders(pid)
      } catch (e) {
        logger.error("design", "DesignView::createFolder", "create folder failed", e)
        toast.error(t("design.err.save", "保存失败"))
      }
    },
    [tx, t, loadFolders],
  )
  const deleteFolder = useCallback(
    async (path: string) => {
      const pid = activeProjectRef.current?.id
      if (!pid) return
      try {
        await tx.call("delete_design_folder_cmd", { projectId: pid, path })
        await Promise.all([loadFolders(pid), loadArtifacts(pid)]) // 页面已移到根
      } catch (e) {
        logger.error("design", "DesignView::deleteFolder", "delete folder failed", e)
        toast.error(t("design.err.save", "保存失败"))
      }
    },
    [tx, t, loadFolders, loadArtifacts],
  )
  const renameFolder = useCallback(
    async (from: string, to: string) => {
      const pid = activeProjectRef.current?.id
      if (!pid) return
      try {
        await tx.call("rename_design_folder_cmd", { projectId: pid, from, to })
        await Promise.all([loadFolders(pid), loadArtifacts(pid)])
      } catch (e) {
        logger.error("design", "DesignView::renameFolder", "rename folder failed", e)
        toast.error(t("design.err.save", "保存失败"))
      }
    },
    [tx, t, loadFolders, loadArtifacts],
  )
  const moveArtifactToFolder = useCallback(
    async (id: string, folder: string) => {
      const pid = activeProjectRef.current?.id
      if (!pid) return
      try {
        await tx.call("move_design_artifact_cmd", { id, folder })
        await Promise.all([loadFolders(pid), loadArtifacts(pid)])
      } catch (e) {
        logger.error("design", "DesignView::moveArtifact", "move failed", e)
        toast.error(t("design.err.save", "保存失败"))
      }
    },
    [tx, t, loadFolders, loadArtifacts],
  )
  // 文件夹随项目/产物变化重载（folder 由产物路径 ∪ 持久化空文件夹派生，产物增删移都可能改动）。
  useEffect(() => {
    const pid = activeProject?.id
    if (pid) void loadFolders(pid)
  }, [activeProject?.id, artifacts, loadFolders])

  const openProject = useCallback(
    (project: DesignProject) => {
      setActiveProject(project)
      setActiveArtifact(null)
      setShowGrid(false)
      setRenamingProject(false)
      setRenamingArtifactId(null)
      void loadArtifacts(project.id, true)
    },
    [loadArtifacts],
  )

  const backToHome = useCallback(() => {
    setActiveProject(null)
    setActiveArtifact(null)
    setArtifacts([])
    void loadProjects()
  }, [loadProjects])

  // 批量删项目（LaunchHome 内已二次确认；此处 settle-all + 汇总提示 + 重载）。
  const batchDeleteProjects = useCallback(
    async (ids: string[]) => {
      if (ids.length === 0) return
      const results = await Promise.allSettled(
        ids.map((id) => tx.call("delete_design_project_cmd", { id })),
      )
      const failed = results.filter((r) => r.status === "rejected").length
      if (activeProject && ids.includes(activeProject.id)) backToHome()
      await loadProjects()
      if (failed > 0) {
        toast.error(t("design.err.batchDeletePartial", "{{n}} 个项目删除失败", { n: failed }))
      } else {
        toast.success(t("design.ok.batchDeleted", "已删除 {{n}} 个项目", { n: ids.length }))
      }
    },
    [tx, t, loadProjects, activeProject, backToHome],
  )

  const createArtifact = useCallback(
    async (kind: ArtifactKind, prompt?: string) => {
      if (!activeProject) return
      try {
        // 有 brief → 走流式生成（返回 generating 壳，内容经 design:generate_delta 回填）；
        // 无 brief 的空白产物走原阻塞建。image 由后端回落阻塞出图。
        const cmd = prompt ? "generate_design_artifact_cmd" : "create_design_artifact_cmd"
        const artifact = await tx.call<DesignArtifact>(cmd, {
          input: {
            projectId: activeProject.id,
            title: kind === "image" && prompt ? prompt.slice(0, 40) : `${kindLabel(kind)}`,
            kind,
            prompt,
          },
        })
        await loadArtifacts(activeProject.id)
        if (artifact) {
          // 关掉产物墙面板：新产物落在根文件夹，若面板正停在某子文件夹里，新建结果既不在
          // 当前面板视图、又被面板盖住单产物预览 = 用户看不到反馈（review MED）。收起面板
          // 直接呈现新产物预览。
          setShowGrid(false)
          void openArtifact(artifact)
        }
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

  // 拖入导入：把拖进来的图片文件建成 image 形态产物（自包含 data-uri）。
  const [dropActive, setDropActive] = useState(false)
  const importImageFiles = useCallback(
    async (files: File[]) => {
      if (!activeProject) return
      const images = files.filter((f) => f.type.startsWith("image/"))
      if (images.length === 0) {
        toast.error(t("design.dropImport.onlyImages", "只能拖入图片文件"))
        return
      }
      let last: DesignArtifact | null = null
      for (const f of images) {
        try {
          const dataB64 = await new Promise<string>((resolve, reject) => {
            const r = new FileReader()
            r.onload = () => {
              const s = String(r.result || "")
              const i = s.indexOf(",")
              resolve(i >= 0 ? s.slice(i + 1) : s)
            }
            r.onerror = () => reject(r.error)
            r.readAsDataURL(f)
          })
          const art = await tx.call<DesignArtifact>("import_design_image_cmd", {
            projectId: activeProject.id,
            title: f.name.replace(/\.[^.]+$/, "").slice(0, 40) || "Imported image",
            mime: f.type,
            dataB64,
          })
          if (art) last = art
        } catch (e) {
          logger.error("design", "DesignView::importImage", "import failed", e)
          toast.error(t("design.dropImport.failed", "导入失败：{{name}}", { name: f.name }))
        }
      }
      await loadArtifacts(activeProject.id)
      if (last) {
        setShowGrid(false)
        void openArtifact(last)
        toast.success(t("design.dropImport.done", "已导入 {{count}} 张图片", { count: images.length }))
      }
    },
    [tx, activeProject, loadArtifacts, openArtifact, t],
  )

  // image 形态需要描述 prompt → 弹小对话框收集。
  const [imagePromptOpen, setImagePromptOpen] = useState(false)
  const [imagePrompt, setImagePrompt] = useState("")
  const [creatingImage, setCreatingImage] = useState(false)
  const [promptKind, setPromptKind] = useState<ArtifactKind>("image")
  const onPickKind = useCallback(
    (kind: ArtifactKind) => {
      // image / audio 是媒体形态：需要一段描述（图像描述 / 旁白文本或音乐提示）→ 收集 prompt。
      if (kind === "image" || kind === "audio") {
        setPromptKind(kind)
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
      await createArtifact(promptKind, imagePrompt.trim())
      setImagePromptOpen(false) // only on success — createArtifact throws on failure
    } catch {
      // error already surfaced via toast in createArtifact; keep dialog open to retry
    } finally {
      setCreatingImage(false)
    }
  }, [createArtifact, imagePrompt, promptKind])

  // ── 从参考图生成匹配产物（「照着这张图做」）─────────────────
  const [refDialogOpen, setRefDialogOpen] = useState(false)
  const [refKind, setRefKind] = useState<ArtifactKind>("web")
  const [refImage, setRefImage] = useState<{ b64: string; mime: string; url: string } | null>(null)
  const [refExtra, setRefExtra] = useState("")
  const [refGenerating, setRefGenerating] = useState(false)

  // 客户端**自适应降采样 + 压到字节预算**再 base64：逐步降边长(1600→…)+ 质量(0.85→0.55)，
  // 保证 payload 稳在服务端 16 MiB body 限内、上传快（后端 downscale_for_vision 再兜一次）；
  // 任何读取 / 编码失败给明确 toast（不静默留空）。
  const onPickRefImage = useCallback(
    (file: File | null) => {
      if (!file || !file.type.startsWith("image/")) return
      const fail = () => toast.error(t("design.fromImageReadErr", "无法读取该图片，请换一张"))
      const BUDGET = 4_000_000 // base64 字符数上限（≈4 MB，远低于服务端 16 MiB）
      const img = new window.Image()
      const objUrl = URL.createObjectURL(file)
      img.onload = () => {
        URL.revokeObjectURL(objUrl)
        let edge = 1600
        for (let attempt = 0; attempt < 4; attempt++) {
          let w = img.naturalWidth || img.width
          let h = img.naturalHeight || img.height
          if (Math.max(w, h) > edge) {
            const s = edge / Math.max(w, h)
            w = Math.round(w * s)
            h = Math.round(h * s)
          }
          const canvas = document.createElement("canvas")
          canvas.width = w
          canvas.height = h
          const ctx = canvas.getContext("2d")
          if (!ctx) return fail()
          ctx.drawImage(img, 0, 0, w, h)
          for (const q of [0.85, 0.7, 0.55]) {
            let url: string
            try {
              url = canvas.toDataURL("image/jpeg", q)
            } catch {
              return fail()
            }
            const b64 = url.split(",")[1] || ""
            if (b64 && b64.length <= BUDGET) {
              setRefImage({ b64, mime: "image/jpeg", url })
              return
            }
          }
          edge = Math.round(edge * 0.75) // 仍超预算 → 再缩边长重试
        }
        fail() // 4 轮仍超预算（极端大图）
      }
      img.onerror = () => {
        URL.revokeObjectURL(objUrl)
        fail()
      }
      img.src = objUrl
    },
    [t],
  )

  const createFromReferenceImage = useCallback(async () => {
    if (!activeProject || !refImage) return
    setRefGenerating(true)
    try {
      const artifact = await tx.call<DesignArtifact>("generate_design_artifact_cmd", {
        input: {
          projectId: activeProject.id,
          title: kindLabel(refKind),
          kind: refKind,
          referenceImageB64: refImage.b64,
          referenceImageMime: refImage.mime,
          prompt: refExtra.trim() || undefined,
        },
      })
      setRefDialogOpen(false)
      setRefImage(null)
      setRefExtra("")
      await loadArtifacts(activeProject.id)
      if (artifact) void openArtifact(artifact)
    } catch (e) {
      logger.error("design", "DesignView::createFromReferenceImage", "generate from image failed", e)
      toast.error(t("design.fromImageErr", "从参考图生成失败"))
    } finally {
      setRefGenerating(false)
    }
  }, [tx, activeProject, refImage, refKind, refExtra, kindLabel, loadArtifacts, openArtifact, t])

  // ── Prompt-first launch (home hero → generate) ───────────────

  const [homePrompt, setHomePrompt] = useState("")
  const [homeKind, setHomeKind] = useState<ArtifactKind>("web")
  const [homeSystemId, setHomeSystemId] = useState<string | null>(null)
  // 首屏选中的 recipe（模板）id：点模板卡时置入，随生成传给后端让「选不同模板产出可辨差异」。
  // 后端按 (id, kind) 匹配、不匹配即回退，故换 kind 无需清空。
  const [homeRecipeId, setHomeRecipeId] = useState<string | null>(null)
  const [generatingHome, setGeneratingHome] = useState(false)

  // 首屏「一句话 → 生成」：建项目 → 带 prompt 建产物（后端一次模型生成完整自包含设计）→ 打开。
  // 需求补全交给设计 Agent 在对话里按需追问（ask_user_question 的 discovery / direction-cards），
  // 首屏只收一句话，不再叠加静态简报表单。
  const generateFromHome = useCallback(async () => {
    const base = homePrompt.trim()
    if (!base || generatingHome) return
    const prompt = base
    const systemId = homeSystemId ?? designConfig?.defaultSystemId ?? undefined
    let createdProjectId: string | null = null
    setGeneratingHome(true)
    try {
      const project = await tx.call<DesignProject>("create_design_project_cmd", {
        input: { title: base.slice(0, 40) },
      })
      createdProjectId = project.id
      // 首屏一句话 → 流式生成（返回 generating 壳，前端挂稳定 iframe 后逐帧灌入）。
      const artifact = await tx.call<DesignArtifact>("generate_design_artifact_cmd", {
        input: {
          projectId: project.id,
          title: kindLabel(homeKind),
          kind: homeKind,
          prompt,
          systemId,
          recipeId: homeRecipeId ?? undefined,
        },
      })
      setHomePrompt("")
      setHomeRecipeId(null)
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
    homeRecipeId,
    generatingHome,
    designConfig,
    kindLabel,
    openProject,
    openArtifact,
    t,
  ])

  // 品牌包：一句话 → 建项目 → 批量生成一组共享系统的协调产物（形态由弹窗自选）。
  const generateBrandPackFromHome = useCallback(async (kinds: ArtifactKind[]) => {
    const base = homePrompt.trim()
    if (!base || generatingHome || kinds.length === 0) return
    const systemId = homeSystemId ?? designConfig?.defaultSystemId ?? undefined
    let createdProjectId: string | null = null
    setGeneratingHome(true)
    const tid = toast.loading(t("design.brandPack.generating", "正在生成品牌包（多个产物，请稍候）…"))
    // 逐件进度：后端每开始一件 emit 一次，把「一直转圈」换成「正在生成 演示（2/3）」。
    const unlisten = tx.listen("design:brand_pack_progress", (raw) => {
      const p = parsePayload<{ index?: number; total?: number; kind?: string; done?: boolean }>(raw)
      if (!p || p.done) return
      if (p.kind && p.index && p.total) {
        toast.loading(
          t("design.brandPack.progress", "正在生成 {{kind}}（{{index}}/{{total}}）…", {
            kind: kindLabel(p.kind as ArtifactKind),
            index: p.index,
            total: p.total,
          }),
          { id: tid },
        )
      }
    })
    try {
      const project = await tx.call<DesignProject>("create_design_project_cmd", {
        input: { title: base.slice(0, 40) },
      })
      createdProjectId = project.id
      const arts = await tx.call<DesignArtifact[]>("generate_design_brand_pack_cmd", {
        projectId: project.id,
        brief: base,
        kinds,
        systemId,
      })
      setHomePrompt("")
      setHomeRecipeId(null)
      openProject(project)
      if (arts && arts.length > 0) void openArtifact(arts[0])
      toast.success(t("design.brandPack.done", "已生成 {{count}} 个产物", { count: arts?.length ?? 0 }), {
        id: tid,
      })
    } catch (e) {
      logger.error("design", "DesignView::brandPack", "brand pack failed", e)
      toast.error(t("design.err.create", "创建失败"), { id: tid })
      if (createdProjectId) {
        try {
          await tx.call("delete_design_project_cmd", { id: createdProjectId })
        } catch {
          /* best effort */
        }
      }
    } finally {
      unlisten()
      setGeneratingHome(false)
    }
  }, [
    tx,
    homePrompt,
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
    async (patch: {
      oid: number
      styles?: [string, string][]
      text?: string
      attrs?: [string, string][]
      remove?: boolean
    }) => {
      if (!activeArtifact) return false
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
        return true
      } catch (e) {
        // stale write or error → hard reload to resync; clear the now-invalid
        // selection and tell the user to re-pick (oid may no longer match).
        suppressReloadRef.current = false
        setPreviewKey((k) => k + 1)
        setSelected(null)
        logger.error("design", "DesignView::commitPatch", "patch failed", e)
        toast.error(t("design.staleReselect", "源已更新，请重新选择元素后再试"))
        return false
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
  // 删除元素（Wave 3-⑫）：确定性 remove patch + 关面板 + 重载反映结构变化。最后一个元素后端拒。
  const handleDeleteElement = useCallback(async () => {
    const oid = selectedRef.current?.oid
    if (oid == null) return
    setSelected(null)
    const ok = await commitPatch({ oid: Number(oid), remove: true })
    if (ok) setPreviewKey((k) => k + 1)
  }, [commitPatch])
  const handleCommitStyle = useCallback(
    (prop: string, value: string) => {
      const oid = selectedRef.current?.oid
      if (oid == null) return
      // 先 live-apply 到 iframe：commitPatch 会抑制重挂，否则 commit-only 控件（字号/间距/布局/
      // 尺寸/描边/阴影…）提交后预览无变化（review #1）。
      postToIframe({ type: "ds_preview_style", oid, props: [[prop, value]] })
      const before = selectedRef.current?.styles?.[prop] ?? ""
      // 乐观刷新 selected.styles：让派生控件（isFlexish / display·align Select 值 / 不透明度）
      // 立即反映本次提交，不等重选（review #3）。
      setSelected((prev) =>
        prev ? { ...prev, styles: { ...prev.styles, [prop]: value } } : prev,
      )
      pushHistoryRef.current({
        oid: Number(oid),
        before: { styles: [[prop, before]] },
        after: { styles: [[prop, value]] },
      })
      void commitPatch({ oid: Number(oid), styles: [[prop, value]] })
    },
    [commitPatch, postToIframe],
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
      const before = selectedRef.current?.text ?? ""
      pushHistoryRef.current({ oid: Number(oid), before: { text: before }, after: { text } })
      void commitPatch({ oid: Number(oid), text })
    },
    [commitPatch],
  )

  // ── 可视化编辑 undo/redo（B5：inverse-patch 栈，客户端）─────────
  // 每次 commit 记 {oid, before, after}（before 值来自 selected 的当前/计算值），undo=回放 before、
  // redo=回放 after，均经确定性 patch 引擎（视觉等价；样式从「未显式设」回退为计算值、无害）。
  const [undoStack, setUndoStack] = useState<EditOp[]>([])
  const [redoStack, setRedoStack] = useState<EditOp[]>([])
  // 镜像栈到 ref，让串行化的 runHistoryStep 读到当前值而不进 deps（避免 keydown 监听反复重挂）。
  const undoStackRef = useRef<EditOp[]>([])
  const redoStackRef = useRef<EditOp[]>([])
  undoStackRef.current = undoStack
  redoStackRef.current = redoStack
  const pushHistory = useCallback((op: EditOp) => {
    // undo/redo 经 commitPatch 直提交、不走 commit handlers，故不会触发 pushHistory —— 无需
    // 「正在回放」守卫（旧守卫在 undo 的 async 窗口内会误吞用户此刻的真实编辑，review 修复 #6）。
    setUndoStack((s) => [...s.slice(-49), op]) // 上限 50，防无界增长
    setRedoStack([])
  }, [])
  // 让 commit handlers 引用最新 pushHistory（ref 提前声明，此处赋值）。
  pushHistoryRef.current = pushHistory

  const applyPayloadLive = useCallback(
    (oid: number, p: PatchPayload) => {
      if (p.styles) for (const [k, v] of p.styles) postToIframe({ type: "ds_preview_style", oid, props: [[k, v]] })
      if (p.text != null) postToIframe({ type: "ds_set_text", oid, text: p.text })
      if (p.attrs) postToIframe({ type: "ds_preview_attr", oid, attrs: p.attrs })
    },
    [postToIframe],
  )
  // undo/redo 单步：**串行化 + 提交成功后才移栈**（review 修复）。
  // ① `historyBusyRef` 防并发/连按（键盘自动重复）——commit 在途时后续按键忽略，
  //    保证下一步用的是 refreshView 之后的新 bodyHash（否则同一 stale hash 触发 stale-write 全拒）。
  // ② 一切副作用（live 预览 / setSelected / commit / 移栈）都在 updater **之外**（updater 须纯，
  //    StrictMode 双调不再双跑 commit）。③ commit 失败（stale 等）**不移栈**，历史与磁盘不脱节。
  const historyBusyRef = useRef(false)
  const runHistoryStep = useCallback(
    async (which: "undo" | "redo") => {
      if (historyBusyRef.current) return
      const stack = which === "undo" ? undoStackRef.current : redoStackRef.current
      if (stack.length === 0) return
      const op = stack[stack.length - 1]
      const payload = which === "undo" ? op.before : op.after
      historyBusyRef.current = true
      applyPayloadLive(op.oid, payload)
      setSelected((prev) => {
        if (!prev || Number(prev.oid) !== op.oid) return prev
        const next = { ...prev, styles: { ...prev.styles }, attrs: { ...(prev.attrs ?? {}) } }
        if (payload.styles) for (const [k, v] of payload.styles) next.styles[k] = v
        if (payload.text != null) next.text = payload.text
        if (payload.attrs) for (const [k, v] of payload.attrs) next.attrs[k] = v
        return next
      })
      const ok = await commitPatch({ oid: op.oid, ...payload })
      historyBusyRef.current = false
      if (!ok) return // 提交失败：保持栈不动（不脱节）
      // **按身份移栈**（review 修复）：commit 的 await 窗口内若有并发 live 检视器编辑
      //（`historyBusyRef` 只串行 undo/redo，不挡 `handleCommitStyle`）会向 undoStack 顶
      // push 新 op；此时按位置 `slice(0,-1)` 会误删那条新编辑而非本次撤销的 `op`，令内存历史
      // 与磁盘脱节。改按对象身份 filter 掉 `op`（EditOp 每次新建、引用唯一），并发编辑安然留栈。
      if (which === "undo") {
        setUndoStack((s) => s.filter((x) => x !== op))
        setRedoStack((r) => [...r, op])
      } else {
        setRedoStack((r) => r.filter((x) => x !== op))
        setUndoStack((s) => [...s, op])
      }
    },
    [applyPayloadLive, commitPatch],
  )
  const undo = useCallback(() => void runHistoryStep("undo"), [runHistoryStep])
  const redo = useCallback(() => void runHistoryStep("redo"), [runHistoryStep])
  // 清空历史：切产物时（oid 空间变、旧 op 不再适用）。
  useEffect(() => {
    setUndoStack([])
    setRedoStack([])
  }, [activeArtifactId])
  // Cmd/Ctrl+Z 撤销 / Cmd/Ctrl+Shift+Z 重做——但焦点在输入框 / contenteditable 时让位原生撤销。
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.key.toLowerCase() !== "z") return
      // 画框批注期 Cmd/Ctrl+Z 归叠层自己的 mark undo（其监听后注册、无法阻断本 window sibling
      // 监听）；不加此守卫会连带回退上一次可视化编辑并落盘（review HIGH：静默数据篡改）。
      if (drawModeRef.current) return
      const ae = document.activeElement as HTMLElement | null
      const tag = ae?.tagName
      if (tag === "INPUT" || tag === "TEXTAREA" || ae?.isContentEditable) return
      e.preventDefault()
      if (e.shiftKey) redo()
      else undo()
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [undo, redo])

  // ── B5：链接 / 图片属性编辑 ──
  const handleLiveAttr = useCallback(
    (attr: string, value: string) => {
      const oid = selectedRef.current?.oid
      if (oid == null) return
      postToIframe({ type: "ds_preview_attr", oid, attrs: [[attr, value]] })
    },
    [postToIframe],
  )
  const handleCommitAttr = useCallback(
    (attr: string, value: string) => {
      const oid = selectedRef.current?.oid
      if (oid == null) return
      const before = selectedRef.current?.attrs?.[attr] ?? ""
      postToIframe({ type: "ds_preview_attr", oid, attrs: [[attr, value]] })
      setSelected((prev) =>
        prev ? { ...prev, attrs: { ...(prev.attrs ?? {}), [attr]: value } } : prev,
      )
      pushHistoryRef.current({
        oid: Number(oid),
        before: { attrs: [[attr, before]] },
        after: { attrs: [[attr, value]] },
      })
      void commitPatch({ oid: Number(oid), attrs: [[attr, value]] })
    },
    [commitPatch, postToIframe],
  )
  // 选本地图 → data-uri（fetch src→blob→canvas 降采样，统一桌面/HTTP；Tauri 无 File 也走 src fetch）。
  const handlePickImage = useCallback(async (): Promise<string | null> => {
    let picked: Awaited<ReturnType<typeof tx.pickLocalImage>> = null
    try {
      picked = await tx.pickLocalImage()
      if (!picked?.src) return null
      return await imageToDataUri(picked.src)
    } catch (e) {
      logger.error("design", "DesignView::handlePickImage", "pick image failed", e)
      toast.error(t("design.err.load", "加载失败"))
      return null
    } finally {
      // 无论成功 / 抛错都释放 objectURL（review 修复 #7：失败路径原会泄漏 blob: URL）。
      picked?.revoke?.()
    }
  }, [tx, t])

  // ── 批注钉 handlers ──
  const loadComments = useCallback(async () => {
    const aid = activeArtifactRef.current?.id
    if (!aid) return
    try {
      const list = await tx.call<DesignComment[]>("design_comment_list_cmd", { artifactId: aid })
      setComments(Array.isArray(list) ? list : [])
    } catch (e) {
      logger.error("design", "DesignView::loadComments", "load comments failed", e)
    }
  }, [tx])

  const handleCreateComment = useCallback(
    async (body: string) => {
      const aid = activeArtifactRef.current?.id
      const p = pendingPlacement
      if (!aid || !p) return
      // 套选批注（Wave 2-⑪）：把成员清单作范围前缀写进正文，随批注带给 AI（snippet 留作 pin 锚点）。
      const finalBody =
        p.members && p.members.length > 1
          ? `${t("design.comment.lassoScope", "套选 {{n}} 个：{{list}}", {
              n: p.members.length,
              list: p.members
                .map((m) => (m.snippet ? `${m.tag}「${m.snippet.slice(0, 16)}」` : m.tag))
                .join("、"),
            })}\n${body}`
          : body
      try {
        await tx.call("design_comment_add_cmd", {
          artifactId: aid,
          oid: p.oid,
          relX: p.relX,
          relY: p.relY,
          tag: p.tag,
          snippet: p.snippet,
          body: finalBody,
        })
        setPendingPlacement(null)
        await loadComments()
      } catch (e) {
        logger.error("design", "DesignView::createComment", "add comment failed", e)
        toast.error(t("design.comment.addFailed", "添加批注失败"))
      }
    },
    [tx, pendingPlacement, loadComments, t],
  )

  const handleResolveComment = useCallback(
    async (id: number, resolved: boolean) => {
      const aid = activeArtifactRef.current?.id
      if (!aid) return
      try {
        await tx.call("design_comment_resolve_cmd", { artifactId: aid, commentId: id, resolved })
        await loadComments()
      } catch (e) {
        logger.error("design", "DesignView::resolveComment", "resolve failed", e)
      }
    },
    [tx, loadComments],
  )

  const handleEditComment = useCallback(
    async (id: number, body: string) => {
      const aid = activeArtifactRef.current?.id
      if (!aid) return
      try {
        await tx.call("design_comment_update_cmd", { artifactId: aid, commentId: id, body })
        await loadComments()
      } catch (e) {
        logger.error("design", "DesignView::editComment", "edit failed", e)
      }
    },
    [tx, loadComments],
  )

  const handleDeleteComment = useCallback(
    async (id: number) => {
      const aid = activeArtifactRef.current?.id
      if (!aid) return
      try {
        await tx.call("design_comment_delete_cmd", { artifactId: aid, commentId: id })
        await loadComments()
      } catch (e) {
        logger.error("design", "DesignView::deleteComment", "delete failed", e)
      }
    },
    [tx, loadComments],
  )

  const handleRelocateComment = useCallback(
    async (id: number, oid: number | null, relX: number, relY: number) => {
      const aid = activeArtifactRef.current?.id
      if (!aid) return
      try {
        await tx.call("design_comment_relocate_cmd", { artifactId: aid, commentId: id, oid, relX, relY })
        await loadComments()
      } catch (e) {
        logger.error("design", "DesignView::relocateComment", "relocate failed", e)
      }
    },
    [tx, loadComments],
  )

  // 批注带到对话（批注 → composer quote chip，用户可补充后随 turn 发，AI 在完整对话
  // 上下文下迭代）。展开被折叠的对话栏并把反馈作为可删 quote 塞进 composer。
  const handleAddCommentToChat = useCallback(
    async (id: number) => {
      const c = comments.find((x) => x.id === id)
      if (!c) return
      const label = c.snippet?.trim()
        ? `${t("design.comment.title", "批注")} · ${c.snippet.trim().slice(0, 40)}`
        : t("design.comment.title", "批注")
      const context = c.snippet?.trim() ? `元素「${c.snippet.trim()}」` : "选中的元素"
      // 锚定元素时把 oid + 硬范围提示一并给 AI：让它用 design 的 edit_element(oid) 就地精改这一个
      // 元素、保留其它一切，而不是整段重造（内容被抹空的根因）。脱锚（oid 空）则只带反馈文字。
      const anchored = c.oid != null
      // 增量（[8]）：带上该元素**当前 computedStyle** 富化 scope，省模型一次 get_artifact；跨源 round-trip。
      const styleLine = anchored ? formatStyleLine((await requestElementStyles([c.oid as number]))[String(c.oid)]) : ""
      const content = anchored
        ? `针对${context}（oid=${c.oid}）的反馈：${c.body}${styleLine}\n` +
          `请只改这一个元素：用 design 工具 edit_element(oid=${c.oid}, style/text/...) 就地精改，` +
          `保留其它一切；不确定当前样式先 get_artifact 读 source。别为这点改动重造整个产物。`
        : `针对${context}的反馈：${c.body}`
      enqueueChatQuote({
        path: `design-comment:${id}`,
        name: label,
        startLine: 0,
        endLine: 0,
        content,
      })
    },
    [comments, t, enqueueChatQuote, requestElementStyles],
  )

  // 编辑模式选中元素 → 一键带到对话（不必先进批注模式）。复用 comment→chat 的同一 quote 注入，
  // 但源取自 editMode 的 selected（含 oid/tag/text）：把 oid + 硬范围提示带给 AI，用户在 composer
  // 里补充「怎么改」后随 turn 发。oid 与 get_artifact / edit_element 同一确定性编号，命中同一元素。
  const handleAddSelectedToChat = useCallback(() => {
    const el = selectedRef.current
    if (!el) return
    const snippet = el.text?.trim().slice(0, 40)
    const context = snippet ? `元素「${snippet}」` : `<${el.tag}>`
    const label = snippet ? `<${el.tag}> · ${snippet}` : `<${el.tag}>`
    const content =
      `已选中${context}（oid=${el.oid}）。请只改这一个元素：用 design 工具 ` +
      `edit_element(oid=${el.oid}, style/text/...) 就地精改，保留其它一切；不确定当前样式先 ` +
      `get_artifact 读 source。别为这点改动重造整个产物。`
    enqueueChatQuote({
      path: `design-element:${el.oid}`,
      name: label,
      startLine: 0,
      endLine: 0,
      content,
    })
    toast.success(t("design.insp.addedToChat", "已加入对话，去补充你的要求"))
  }, [t, enqueueChatQuote])

  // 右键菜单：任意点击 / 滚动即关（同 MessageList contextMenu 范式）。
  useEffect(() => {
    if (!previewCtxMenu) return
    const close = () => setPreviewCtxMenu(null)
    document.addEventListener("mousedown", close)
    document.addEventListener("scroll", close, true)
    return () => {
      document.removeEventListener("mousedown", close)
      document.removeEventListener("scroll", close, true)
    }
  }, [previewCtxMenu])
  // 右键菜单「添加批注」：切到批注模式并以右键选中元素为锚直接开待填钉（锚点取元素中心）。
  const handleCtxAddComment = useCallback(() => {
    const el = selectedRef.current
    if (!el) return
    setEditMode(false)
    setCommentMode(true)
    setPendingPlacement({
      oid: el.oid != null ? Number(el.oid) : null,
      relX: 0.5,
      relY: 0.5,
      tag: el.tag,
      snippet: el.text?.trim().slice(0, 60) || undefined,
    })
  }, [])
  // 右键菜单「复制文本」：编辑态右键被接管后补偿原生最常用需求。
  const handleCtxCopyText = useCallback(() => {
    const txt = selectedRef.current?.text?.trim()
    if (!txt) return
    void navigator.clipboard
      .writeText(txt)
      .then(() => toast.success(t("design.ctx.copied", "已复制")))
      .catch(() => toast.error(t("design.err.load", "加载失败")))
  }, [t])

  // 批量带到对话（B4-2）：多条批注合成一个 scope-guarded 结构块（编号 + 元素 + 反馈），
  // 作为单条 quote 塞进 composer——对齐参照 <attached-preview-comments> 的「硬范围」约束。
  const handleBatchCommentsToChat = useCallback(
    async (ids: number[]) => {
      const chosen = ids
        .map((id) => comments.find((x) => x.id === id))
        .filter((c): c is (typeof comments)[number] => !!c)
      if (chosen.length === 0) return
      // 增量（[8]）：批量取所有锚定 oid 的当前 computedStyle，逐条富化 scope（省模型 get_artifact）。
      const anchoredOids = chosen.map((c) => c.oid).filter((o): o is number => o != null)
      const styleMap = anchoredOids.length ? await requestElementStyles(anchoredOids) : {}
      const lines = chosen
        .map((c, i) => {
          const el = c.snippet?.trim()
            ? `元素「${c.snippet.trim()}」`
            : c.tag
              ? `<${c.tag}>`
              : t("design.comment.title", "批注")
          // 锚定元素带上 oid + 当前样式，供 AI 对每条用 edit_element(oid) 就地精改。
          const oidTag = c.oid != null ? `（oid=${c.oid}）` : ""
          const styleLine = c.oid != null ? formatStyleLine(styleMap[String(c.oid)]) : ""
          return `${i + 1}. ${el}${oidTag}：${c.body}${styleLine}`
        })
        .join("\n")
      const content =
        `${t(
          "design.comment.batchScopeHint",
          "请仅修改下列被标注的元素，其它保持不变：",
        )}\n${lines}\n` +
        `逐条用 design 工具 edit_element(oid, style/text/...) 就地精改，不确定当前样式先 get_artifact 读 source；别重造整个产物。`
      enqueueChatQuote({
        path: `design-comments:${ids.slice().sort((a, b) => a - b).join("-")}`,
        name: t("design.comment.batchLabel", "{{count}} 条批注", { count: chosen.length }),
        startLine: 0,
        endLine: 0,
        content,
      })
    },
    [comments, t, enqueueChatQuote, requestElementStyles],
  )

  // 反-slop 自查复查（B0-2）：recheck 对当前正文重跑自查、dismiss 用户判定无碍强制清标记。
  const handleReviewArtifact = useCallback(
    async (action: "recheck" | "dismiss") => {
      const aid = activeArtifactRef.current?.id
      if (!aid) return
      try {
        const updated = await tx.call<DesignArtifact>("design_review_artifact_cmd", {
          artifactId: aid,
          action,
        })
        await openArtifact(updated) // 取全视图（含预览路径）+ 刷新 status/metadata
        if (activeProjectRef.current) void loadArtifacts(activeProjectRef.current.id)
        toast.success(
          action === "dismiss"
            ? t("design.review.dismissed", "已标记为已复查")
            : t("design.review.rechecked", "已重新检查"),
        )
      } catch (e) {
        logger.error("design", "DesignView::reviewArtifact", "review failed", e)
        toast.error(t("design.err.load", "加载失败"))
      }
    },
    [tx, openArtifact, loadArtifacts, t],
  )

  // 「让 AI 修复」：把当前产物自查发现的问题作为一条针对性指令直接发进设计对话——
  // Agent 走 get_artifact → edit/restyle 就地修，修完 edit 路径重跑确定性自查自动清徽章，
  // 无需用户再手动 recheck。面板折叠则缓冲、打开后 flush（见 pendingFixRef effect）。
  const handleFixWithAgent = useCallback(() => {
    const art = activeArtifactRef.current
    if (!art) return
    const detail =
      parseSelfCheck(art.metadata)?.detail ??
      t("design.review.flagged", "自查发现可能的质量问题，建议复查")
    const prompt = t(
      "design.review.fixPrompt",
      "请修复设计自查发现的问题：{{detail}}。用设计系统 token（var(--ds-*)）收敛配色、替换硬编码样式，只做必要的最小改动，保持产物其它内容与整体设计不变。",
      { detail },
    )
    setChatOpen(true)
    if (chatPanelRef.current) {
      const sent = chatPanelRef.current.submitPrompt(prompt)
      if (sent) toast.success(t("design.review.fixStarted", "已让 AI 开始修复"))
      else toast.error(t("design.review.fixBusy", "对话进行中，请稍候再试"))
    } else {
      pendingFixRef.current = prompt
      toast.success(t("design.review.fixStarted", "已让 AI 开始修复"))
    }
  }, [t])

  // 多镜头质量审查（确定性 a11y / 内容 / 语义）：owner 按需跑，弹结果列表。
  const [reviewFindings, setReviewFindings] = useState<
    { lens: string; severity: string; message: string }[] | null
  >(null)
  const [reviewing, setReviewing] = useState(false)
  const runQualityReview = useCallback(async () => {
    const aid = activeArtifactRef.current?.id
    if (!aid || reviewing) return
    setReviewing(true)
    try {
      const f = await tx.call<{ lens: string; severity: string; message: string }[]>(
        "review_design_artifact_cmd",
        { id: aid },
      )
      setReviewFindings(Array.isArray(f) ? f : [])
    } catch (e) {
      logger.error("design", "DesignView::qualityReview", "review failed", e)
      toast.error(t("design.err.load", "加载失败"))
    } finally {
      setReviewing(false)
    }
  }, [tx, reviewing, t])

  // 页面级样式（body 背景/文字色/最大宽度）：与 oid 元素微调正交，落 CSS 标记块 + 重渲染。
  const [pageStyleOpen, setPageStyleOpen] = useState(false)
  const [psBackground, setPsBackground] = useState("")
  const [psColor, setPsColor] = useState("")
  const [psMaxWidth, setPsMaxWidth] = useState("")
  const [psSaving, setPsSaving] = useState(false)
  const savePageStyle = useCallback(async () => {
    const aid = activeArtifactRef.current?.id
    if (!aid || psSaving) return
    setPsSaving(true)
    try {
      const props: Record<string, string> = {}
      if (psBackground.trim()) props["background"] = psBackground.trim()
      if (psColor.trim()) props["color"] = psColor.trim()
      if (psMaxWidth.trim()) props["max-width"] = psMaxWidth.trim()
      // 空值属性会被后端移除；全空 = 清除页面样式块。
      await tx.call("patch_design_page_style_cmd", { id: aid, props })
      setPageStyleOpen(false)
      toast.success(t("design.pageStyle.saved", "已应用页面样式"))
    } catch (e) {
      logger.error("design", "DesignView::pageStyle", "save failed", e)
      toast.error(t("design.err.load", "加载失败"))
    } finally {
      setPsSaving(false)
    }
  }, [tx, psBackground, psColor, psMaxWidth, psSaving, t])

  // RTL 切换：翻转产物文本方向（存 metadata.dir + 后端重渲染，design:reload 刷新预览）。
  const toggleRtl = useCallback(async () => {
    const a = activeArtifactRef.current
    if (!a) return
    const next = !parseIsRtl(a.metadata)
    try {
      const updated = await tx.call<DesignArtifact>("set_design_artifact_dir_cmd", {
        id: a.id,
        rtl: next,
      })
      await openArtifact(updated)
      toast.success(
        next ? t("design.rtl.on", "已切换为从右到左（RTL）") : t("design.rtl.off", "已切换为从左到右（LTR）"),
      )
    } catch (e) {
      logger.error("design", "DesignView::toggleRtl", "set dir failed", e)
      toast.error(t("design.err.load", "加载失败"))
    }
  }, [tx, openArtifact, t])

  // 回灌对话：让 AI 按批注精修产物（一键快捷路径）。design-space 原生——产物就地更新新版本、
  // 无需切走；`design:reload` 事件自动刷新预览。
  const handleSendCommentToChat = useCallback(
    async (id: number) => {
      const aid = activeArtifactRef.current?.id
      if (!aid) return
      const p = tx.call("design_comment_refine_cmd", { artifactId: aid, commentId: id })
      toast.promise(p, {
        loading: t("design.comment.refining", "AI 正在按批注精修…"),
        success: t("design.comment.refined", "已按批注精修，查看新版本"),
        error: (e: unknown) =>
          e instanceof Error ? e.message : t("design.comment.refineFailed", "精修失败"),
      })
      try {
        await p
        await refreshView()
      } catch (e) {
        logger.error("design", "DesignView::refineComment", "refine failed", e)
      }
    },
    [tx, t, refreshView],
  )

  // 载入 / 清空批注：进批注模式或切产物时拉取；退出清空。
  useEffect(() => {
    if (commentMode && activeArtifactRef.current) void loadComments()
    else setComments([])
    setPendingPlacement(null)
  }, [commentMode, activeArtifact?.id, loadComments])

  // 同步批注模式 + 数据到 iframe（钉由 bridge 渲染）。
  useEffect(() => {
    postToIframe({ type: "ds_comment_mode", on: commentMode })
  }, [commentMode, postToIframe])
  useEffect(() => {
    if (commentMode) postToIframe({ type: "ds_comments_set", comments })
  }, [comments, commentMode, postToIframe])
  // 待填钉解析（保存 / 取消 / 复位任一路径 → pendingPlacement 归 null）时，撤掉 bridge 里
  // 当前待填元素的持久高亮。统一走此处，覆盖全部清空点（切元素时 bridge 自身已换高亮，不受影响）。
  useEffect(() => {
    if (!pendingPlacement) postToIframe({ type: "ds_comment_pending_clear" })
  }, [pendingPlacement, postToIframe])
  // 打开产物时**自愈渲染版本**：inspector bridge 等编辑工具层升级后，老产物 index.html 仍烧着
  // 旧 bridge（bridge 烧死在渲染产物里）。静默用当前 renderer 重渲染（内容不变、不新增版本），
  // 重渲染了就 bump previewKey 重载 iframe。对 ready / needs_review 态都跑——与后端
  // ensure_artifact_render_fresh 放行集一致（needs_review 产物同样有 bridge、可微调，此前前端只放
  // ready 致 slop 标记产物永不自愈老 bridge，工具层修复对它们不生效）；id / status 变化各触发一次。
  useEffect(() => {
    const art = activeArtifactRef.current
    if (!art || (art.status !== "ready" && art.status !== "needs_review")) return
    let cancelled = false
    void tx
      .call<boolean>("ensure_design_artifact_fresh_cmd", { id: art.id })
      .then((rerendered) => {
        if (!cancelled && rerendered) setPreviewKey((k) => k + 1)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [activeArtifactId, activeArtifact?.status, tx])

  // Receive selection from the iframe bridge + stream-host ready handshake.
  useEffect(() => {
    const onMsg = (e: MessageEvent) => {
      // 只信任来自预览 iframe 自身的消息——沙盒（allow-scripts）里 AI 生成/可能被注入的脚本能向
      // parent postMessage，而 host 会据此回写产物源（ds_text_commit 等）。校验 e.source 收窄面。
      // 收窄来源：预览 iframe 或演示态 iframe（deck 演示导航需接收演示 iframe 的 slide 状态）。
      if (
        e.source !== iframeRef.current?.contentWindow &&
        e.source !== presentIframeRef.current?.contentWindow
      )
        return
      const d = e.data as {
        type?: string
        payload?: DesignSelectedElement
        oid?: number | string
        text?: string
        id?: number
        relX?: number
        relY?: number
        tag?: string
        snippet?: string
        x?: number
        y?: number
        active?: number
        count?: number
        members?: { oid: number; tag: string; snippet: string; relX: number; relY: number }[]
        styles?: Record<string, Record<string, string>>
      }
      // 画框批注视口度量回传（B4-1，跨源；resolve 对应 requestViewportMetrics 的 promise）。
      if (d?.type === "ds_viewport_result" && typeof d.id === "number") {
        viewportReqRef.current.get(d.id)?.(e.data as ViewportMetrics)
        return
      }
      // 涂画命中的 oid 成员回传（resolve 对应 requestDrawHits 的 promise）。
      if (d?.type === "ds_draw_hit_result" && typeof d.id === "number") {
        drawHitReqRef.current.get(d.id)?.(Array.isArray(d.members) ? (d.members as DrawMember[]) : [])
        return
      }
      // 批注 oid 的当前 computedStyle 回传（resolve 对应 requestElementStyles 的 promise）。
      if (d?.type === "ds_style_query_result" && typeof d.id === "number") {
        styleReqRef.current.get(d.id)?.(
          (d.styles as Record<string, Record<string, string>>) ?? {},
        )
        return
      }
      if (d?.type === "ds_selected" && d.payload) {
        setSelected(d.payload)
        // iframe 内点击不冒泡到父 document（关菜单的 mousedown 监听收不到）——改点/改选元素时
        // 在此关右键菜单。右键流不受影响：bridge 先发本消息再发 ds_context_menu，同批后者重开。
        setPreviewCtxMenu(null)
      }
      // 编辑态右键菜单：bridge 先发 ds_selected 选中元素、再发本消息带 iframe 内坐标；
      // 换算 = iframe 屏上位置 + 坐标 × 当前预览缩放，再钳进窗口防溢出。
      else if (d?.type === "ds_context_menu" && editModeRef.current) {
        const rect = iframeRef.current?.getBoundingClientRect()
        if (!rect) return
        const s = previewScaleRef.current
        setPreviewCtxMenu({
          x: Math.max(8, Math.min(rect.left + Number(d.x ?? 0) * s, window.innerWidth - 184)),
          y: Math.max(8, Math.min(rect.top + Number(d.y ?? 0) * s, window.innerHeight - 200)),
        })
      }
      // 就地文本编辑提交：双击叶子元素改文案 → 走同一确定性回写（apply_text_patch +
      // expectedHash）。仅编辑态受理；oid 直接来自被编辑元素。
      else if (d?.type === "ds_text_commit" && d.oid != null && editModeRef.current) {
        void commitPatch({ oid: Number(d.oid), text: String(d.text ?? "") })
      }
      // 批注模式点选元素落钉 → 开新钉待填表单（正文在面板里填）。
      else if (d?.type === "ds_comment_place" && commentModeRef.current) {
        setPendingPlacement({
          oid: d.oid != null ? Number(d.oid) : null,
          relX: Number(d.relX ?? 0.5),
          relY: Number(d.relY ?? 0.5),
          tag: d.tag,
          snippet: d.snippet,
        })
      }
      // 套选（Wave 2-⑪）：命中多个成员 → 一条批注锚到首成员、pin 落质心，snippet 汇总成员
      //（成员 tag/文本随批注带给 AI = 定向多元素范围约束）。
      else if (d?.type === "ds_lasso_place" && commentModeRef.current && Array.isArray(d.members)) {
        const members = d.members as { oid: number; tag: string; snippet: string; relX: number; relY: number }[]
        if (members.length === 0) return
        const first = members[0]
        // snippet 取首成员**真实文本**（保 resolveEl 文本软着陆，review LOW）；成员汇总在保存时
        // 写进批注正文带给 AI（见 handleCreateComment）。
        setPendingPlacement({
          oid: first.oid,
          relX: Number(first.relX ?? 0.5),
          relY: Number(first.relY ?? 0.5),
          tag: first.tag,
          snippet: first.snippet,
          members: members.map((m) => ({ oid: m.oid, tag: m.tag, snippet: m.snippet })),
        })
      }
      // 拖拽钉 → 重锚到落点元素（确定性回写 rel 位 + oid）。
      else if (d?.type === "ds_comment_relocate" && d.id != null && commentModeRef.current) {
        void handleRelocateComment(
          d.id,
          d.oid != null ? Number(d.oid) : null,
          Number(d.relX ?? 0.5),
          Number(d.relY ?? 0.5),
        )
      }
      // 点击预览里已有的钉（未拖动）→ 展开批注面板并滚动/高亮该条进入编辑（B0-3，此前死接线）。
      else if (d?.type === "ds_comment_click" && d.id != null) {
        setCommentMode(true)
        setFocusCommentId(Number(d.id))
      }
      // Deck slide 状态上报（Wave 2-⑧）：宿主据此渲染页码/翻页按钮、演示保温。
      else if (d?.type === "ds_slide_state" && typeof d.active === "number") {
        setDeckState({ active: d.active, count: Number(d.count ?? 1) })
      }
      // 滚动保温（Wave 2-⑥）：桥持续上报滚动位置，按当前产物存最新值；重载 onLoad 后回写，
      // 使每轮改稿 / 换系统 / 定稿 swap 不再被打回顶部。返回产物也恢复上次滚动位置。
      else if (d?.type === "ds_scroll") {
        // iframe 内滚动不触发父层 scroll 监听且菜单锚点随内容滚走——开着就关（null→null 会被
        // React bail-out，持续滚动上报无重渲染开销）。
        setPreviewCtxMenu(null)
        // 重载在途（previewLoading）时丢弃上报：换产物瞬间旧文档的晚到滚动（iframe 复用同一
        // contentWindow，源守卫拦不住）可能被记到新产物名下（review LOW）。载完再记正常滚动。
        const aid = activeArtifactRef.current?.id
        if (aid && !previewLoadingRef.current)
          previewScrollRef.current.set(aid, { x: Number(d.x ?? 0), y: Number(d.y ?? 0) })
      }
      // 流式占位页加载完毕 → 补投最新快照（deltas 可能早于 iframe onload 到达）。
      else if (d?.type === "ds_stream_ready") {
        const snap = streamSnapshotRef.current
        if (snap && snap.artifactId === activeArtifactRef.current?.id) {
          postToIframe({ type: "ds_stream_css", css: snap.css })
          postToIframe({ type: "ds_stream_body", html: snap.bodyHtml })
        }
      }
    }
    window.addEventListener("message", onMsg)
    return () => window.removeEventListener("message", onMsg)
  }, [postToIframe, commitPatch, handleRelocateComment, t])

  // Toggle bridge activation with edit mode. 画框批注（父层叠层）需 iframe bridge 关闭，避免
  // 底层 iframe 抢事件 / 出选中框——drawMode 期间强制 ds_deactivate（editMode 已被三态互斥关掉）。
  useEffect(() => {
    postToIframe({ type: editMode && !drawMode ? "ds_activate" : "ds_deactivate" })
    if (!editMode) {
      setSelected(null)
      setPreviewCtxMenu(null)
    }
  }, [editMode, drawMode, postToIframe])

  // Reset edit state when switching artifacts.
  useEffect(() => {
    setEditMode(false)
    setSelected(null)
    setCommentMode(false)
    setDrawMode(false)
    setDeckState(null) // Wave 2-⑧：切产物先清 deck 页码，等新 deck 桥上报（避免残留旧计数）
  }, [activeArtifact?.id])

  // Re-arm bridge + restore selection after an iframe (re)mount.
  const handleIframeLoad = useCallback(() => {
    setPreviewLoading(false) // 新帧就绪 → 撤 spinner 叠层（Wave 2-⑥）
    setPreviewCtxMenu(null) // 重载后旧菜单挂在已失效元素上，关掉
    if (editModeRef.current) postToIframe({ type: "ds_activate" })
    const oid = selectedRef.current?.oid
    if (oid != null) postToIframe({ type: "ds_reselect", oid })
    // 重挂后重发批注模式 + 钉数据（bridge 是全新实例）。
    if (commentModeRef.current) {
      postToIframe({ type: "ds_comment_mode", on: true })
      postToIframe({ type: "ds_comments_set", comments: commentsRef.current })
    }
    // 滚动保温（Wave 2-⑥）：回写该产物重载前 / 上次的滚动位置（无记录=保持顶部）。
    // 延一帧确保桥已就绪接收；新产物首开无记录故天然停在顶部。
    const aid = activeArtifactRef.current?.id
    const saved = aid ? previewScrollRef.current.get(aid) : undefined
    if (saved && (saved.x || saved.y)) {
      requestAnimationFrame(() => postToIframe({ type: "ds_scroll_to", x: saved.x, y: saved.y }))
    }
  }, [postToIframe])

  // ── Export (D3): HTML/MD/ZIP（后端）+ PNG/PDF/PPTX/MP4（客户端栅格化） ──
  type ExportFormat =
    | "html"
    | "md"
    | "zip"
    | "handoff"
    | "png"
    | "pdf"
    | "pptx"
    | "pptx-outline"
    | "video"
  const [exporting, setExporting] = useState<null | ExportFormat>(null)

  // 导出强路依赖门：MP4 需 ffmpeg 编码器、PDF/PNG 需浏览器引擎。未就绪时弹门让用户主动选
  // （下载依赖 / 引导安装 / 用较低保真客户端栅格化），不静默降级。ffmpeg 与 browser 共用一个门。
  type DepStatus = {
    ready: boolean
    source: string
    binaryPath: string | null
    canAutoInstall: boolean
  }
  type ExportDep = "ffmpeg" | "browser"
  const [exportGate, setExportGate] = useState<{
    dep: ExportDep
    status: DepStatus
    base: string
    html: string
    format: ExportFormat
  } | null>(null)
  const [gateInstalling, setGateInstalling] = useState(false)
  const [gateProgress, setGateProgress] = useState<number | null>(null)
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
        // 统一保存出口：桌面弹原生「保存到…」框（记住上次目录）+ 存后可「在文件夹中显示」；
        // 网页走 File System Access 选目录、否则回退浏览器下载。取消保存框 = 静默关 loading。
        const save = async (blob: Blob, name: string) =>
          presentSaveResult(await tx.saveFileAs(blob, name), tx, t, { toastId })
        // Text formats (HTML / Markdown) — backend returns the content directly.
        if (format === "html" || format === "md") {
          const fmt = format === "md" ? "markdown" : "html"
          const res = await tx.call<{ filename: string; mime: string; content: string }>(
            "export_design_artifact_cmd",
            { id: activeArtifact.id, format: fmt },
          )
          if (!res) return
          await save(new Blob([res.content], { type: res.mime }), res.filename || `${base}.${format}`)
          return
        }
        // ZIP — backend assembles a source bundle (base64).
        if (format === "zip") {
          const res = await tx.call<{ zip: string }>("export_design_zip_cmd", {
            artifactId: activeArtifact.id,
          })
          if (!res?.zip) return
          await save(base64ToBlob(res.zip, "application/zip"), `${base}.zip`)
          return
        }
        // Handoff — 代码交付包（index.html + source/ + 多平台 tokens/ + HANDOFF.md，base64 zip）。
        if (format === "handoff") {
          const res = await tx.call<{ filename: string; mime: string; content: string }>(
            "export_design_handoff_cmd",
            { id: activeArtifact.id },
          )
          if (!res?.content) return
          await save(
            base64ToBlob(res.content, res.mime || "application/zip"),
            res.filename || `${base}-handoff.zip`,
          )
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
        // 栅格化分支只负责**产出字节**（native 强路失败才回退客户端）；保存放到分支之后统一
        // 走一次 save()——否则把 save() 塞进 native try 里，一次真实的写盘失败会被「native
        // 引擎失败→客户端回退」的 catch 误吞，白白重跑一遍昂贵的客户端重渲染 + 再弹一次保存框
        // （review MED）。保存失败落到外层 catch = 正确的「导出失败」。
        let out: { blob: Blob; name: string } | null = null
        if (format === "png" || format === "pdf") {
          // PDF/PNG 强路 = 真实浏览器原生捕获（PDF 矢量可选文字 / PNG 全保真）。先预检浏览器
          // 引擎：未就绪则弹门让用户主动选（下载 Chromium runtime / 引导 / 用较低保真客户端）。
          const doc = await tx
            .call<DepStatus>("design_browser_doctor_cmd")
            .catch(() => null)
          if (doc && !doc.ready) {
            setExportGate({ dep: "browser", status: doc, base, html: res.content, format })
            if (toastId !== undefined) toast.dismiss(toastId)
            return
          }
          try {
            const nat = await tx.call<{ data: string; mime: string }>(
              "export_design_native_cmd",
              { id: activeArtifact.id, format },
            )
            out = { blob: base64ToBlob(nat.data, nat.mime), name: `${base}.${format}` }
          } catch (e) {
            logger.error(
              "design",
              "DesignView::handleExport",
              `native ${format} failed after ready engine, using client fallback`,
              e,
            )
            const blob =
              format === "png"
                ? await exportPng(res.content, kind, vw, exportOpts)
                : await exportPdf(res.content, kind, vw, exportOpts)
            out = { blob, name: `${base}.${format}` }
          }
        } else if (format === "pptx") {
          out = {
            blob: await exportPptx(res.content, kind, activeArtifact.title, vw, exportOpts),
            name: `${base}.pptx`,
          }
        } else if (format === "pptx-outline") {
          // 结构化可编辑文本：服务端从 deck HTML 抽大纲组装 pptx（非栅格化）。
          const r = await tx.call<{ pptx: string }>("export_design_pptx_outline_cmd", {
            artifactId: activeArtifact.id,
          })
          const bin = atob(r.pptx)
          const arr = new Uint8Array(bin.length)
          for (let i = 0; i < bin.length; i++) arr[i] = bin.charCodeAt(i)
          out = {
            blob: new Blob([arr], {
              type: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            }),
            name: `${base}.pptx`,
          }
        } else if (format === "video") {
          // MP4 强路 = 真实浏览器逐帧渲染 + ffmpeg 编码，**两个依赖都要**（ffmpeg 编码器 + 浏览器
          // 引擎）。两个都预检，任一未就绪即弹门让用户主动选，不静默降级（缺浏览器时若只检
          // ffmpeg 会在 acquire_backend 处失败后静默回退低保真 WebCodecs）。
          const [ffdoc, brdoc] = await Promise.all([
            tx.call<DepStatus>("design_ffmpeg_doctor_cmd").catch(() => null),
            tx.call<DepStatus>("design_browser_doctor_cmd").catch(() => null),
          ])
          if (ffdoc && !ffdoc.ready) {
            setExportGate({ dep: "ffmpeg", status: ffdoc, base, html: res.content, format: "video" })
            if (toastId !== undefined) toast.dismiss(toastId)
            return
          }
          if (brdoc && !brdoc.ready) {
            setExportGate({ dep: "browser", status: brdoc, base, html: res.content, format: "video" })
            if (toastId !== undefined) toast.dismiss(toastId)
            return
          }
          // 就绪（或探针不可用 → 乐观尝试强路）；强路失败仍回退客户端保证可导出。
          try {
            const nat = await tx.call<{ data: string; mime: string }>(
              "export_design_native_cmd",
              { id: activeArtifact.id, format: "video" },
            )
            out = { blob: base64ToBlob(nat.data, nat.mime), name: `${base}.mp4` }
          } catch (e) {
            logger.error(
              "design",
              "DesignView::handleExport",
              "native video failed after ready ffmpeg, using client WebCodecs fallback",
              e,
            )
            const blob = await exportVideo(res.content, vw, activeArtifact.viewportH, {
              scale: designConfig?.exportScale,
              onProgress,
            })
            out = { blob, name: `${base}.mp4` }
          }
        }
        // 统一保存出口：产出字节后弹保存框；成功/取消提示由 save()→presentSaveResult 逐路给出
        // （含桌面 reveal 动作），写盘失败抛到外层 catch → 正确的「导出失败」。
        if (out) await save(out.blob, out.name)
      } catch (e) {
        logger.error("design", "DesignView::handleExport", `export ${format} failed`, e)
        toast.error(t("design.err.export", "导出失败"), toastId !== undefined ? { id: toastId } : undefined)
      } finally {
        setExporting(null)
      }
    },
    [tx, activeArtifact, exporting, t, designConfig],
  )

  // 导出门：下载缺失依赖（ffmpeg 编码器 / Chromium runtime）后重试对应强路。
  // **全程持 `exporting` 锁**（关模态后仍串行）——否则模态关闭到 await 完成的窗口里工具栏导出
  // 按钮会重新可点，第二次原生导出与本次并发争用全局浏览器单例 → 截错帧 / 关掉对方导出页。
  const gateDownloadAndRetry = useCallback(async () => {
    const g = exportGate
    if (!g || !activeArtifact) return
    setExporting(g.format)
    setGateInstalling(true)
    setGateProgress(null)
    // 阶段一：装依赖——失败才是真·「依赖下载失败」。
    try {
      await tx.call(g.dep === "ffmpeg" ? "design_install_ffmpeg_cmd" : "design_install_browser_cmd")
    } catch (e) {
      logger.error("design", "DesignView::gateInstall", `${g.dep} install failed`, e)
      toast.error(t("design.err.depInstall", "依赖下载失败，请重试或改用较低保真"))
      setGateInstalling(false)
      setGateProgress(null)
      setExporting(null)
      return
    }
    setExportGate(null)
    setGateInstalling(false)
    setGateProgress(null)
    // 阶段二：原生捕获 + 保存——写盘失败 = 「导出失败」并复用 tid 撤 loading（不再误报依赖失败 /
    // 卡住 loading 转圈，review MED）。取消保存框由 presentSaveResult 撤 tid。
    const tid = toast.loading(t("design.exporting", "正在导出…"))
    try {
      const nat = await tx.call<{ data: string; mime: string }>("export_design_native_cmd", {
        id: activeArtifact.id,
        format: g.format === "video" ? "video" : g.format,
      })
      const ext = g.format === "video" ? "mp4" : g.format
      presentSaveResult(
        await tx.saveFileAs(base64ToBlob(nat.data, nat.mime), `${g.base}.${ext}`),
        tx,
        t,
        { toastId: tid },
      )
    } catch (e) {
      logger.error("design", "DesignView::gateInstall", "native export/save failed", e)
      toast.error(t("design.err.export", "导出失败"), { id: tid })
    } finally {
      setExporting(null)
    }
  }, [exportGate, activeArtifact, tx, t])

  // 导出门：用较低保真的客户端栅格化（末位显式可选，非静默默认）。持 `exporting` 锁串行。
  const gateUseClient = useCallback(async () => {
    const g = exportGate
    if (!g || !activeArtifact) return
    setExporting(g.format)
    setExportGate(null)
    const tid = toast.loading(t("design.exporting", "正在导出…"))
    const opts = { scale: designConfig?.exportScale, jpegQuality: designConfig?.exportJpegQuality }
    const save = async (blob: Blob, name: string) =>
      presentSaveResult(await tx.saveFileAs(blob, name), tx, t, { toastId: tid })
    try {
      if (g.format === "video") {
        await save(
          await exportVideo(g.html, activeArtifact.viewportW, activeArtifact.viewportH, {
            scale: designConfig?.exportScale,
          }),
          `${g.base}.mp4`,
        )
      } else if (g.format === "png") {
        await save(await exportPng(g.html, activeArtifact.kind, activeArtifact.viewportW, opts), `${g.base}.png`)
      } else if (g.format === "pdf") {
        await save(await exportPdf(g.html, activeArtifact.kind, activeArtifact.viewportW, opts), `${g.base}.pdf`)
      }
    } catch (e) {
      logger.error("design", "DesignView::gateClient", "client export failed", e)
      toast.error(t("design.err.export", "导出失败"), { id: tid })
    } finally {
      setExporting(null)
    }
  }, [exportGate, activeArtifact, designConfig, tx, t])

  // 依赖下载进度（ffmpeg 与 Chromium 各自的 emit 通道）。
  useEffect(() => {
    const onProg = (raw: unknown) => {
      const p = parsePayload<{ stage?: string; percent?: number }>(raw)
      if (p?.stage === "ready") setGateProgress(100)
      else if (p?.stage === "downloading")
        setGateProgress(typeof p.percent === "number" ? p.percent : null)
    }
    const offs = [
      tx.listen("design:ffmpeg_download_progress", onProg),
      tx.listen("browser:chromium_download_progress", onProg),
    ]
    return () => offs.forEach((f) => f())
  }, [tx])

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
      presentSaveResult(
        await tx.saveFileAs(
          base64ToBlob(res.zip, "application/zip"),
          `${safeFilename(activeProject.title)}.zip`,
        ),
        tx,
        t,
        { toastId },
      )
    } catch (e) {
      logger.error("design", "DesignView::exportProject", "export project failed", e)
      toast.error(t("design.err.export", "导出失败"), { id: toastId })
    } finally {
      setExportingProject(false)
    }
  }, [tx, activeProject, exportingProject, t])

  // 批量导出选中产物为一个 ZIP（Wave 1-③）：一次栅格 / 一个保存框，避免 N 个下载对话框。
  const batchExportArtifacts = useCallback(
    async (ids: string[]) => {
      if (ids.length === 0) return
      const toastId = toast.loading(t("design.exporting", "正在导出…"))
      try {
        const res = await tx.call<{ zip: string }>("export_design_selected_zip_cmd", {
          artifactIds: ids,
        })
        if (!res?.zip) return
        const base = activeProject ? safeFilename(activeProject.title) : "design"
        presentSaveResult(
          await tx.saveFileAs(base64ToBlob(res.zip, "application/zip"), `${base}-${ids.length}.zip`),
          tx,
          t,
          { toastId },
        )
      } catch (e) {
        logger.error("design", "DesignView::batchExport", "batch export failed", e)
        toast.error(t("design.err.export", "导出失败"), { id: toastId })
      }
    },
    [tx, activeProject, t],
  )

  // 产物取出（B3-2，仅桌面）：复制产物目录路径 / 在 Finder 打开。远端无本机路径故不显示。
  const copyArtifactPath = useCallback(async () => {
    const path = activeArtifactRef.current?.artifactPath
    if (!path) return
    try {
      await navigator.clipboard.writeText(path)
      toast.success(t("design.ok.pathCopied", "已复制路径"))
    } catch (e) {
      logger.error("design", "DesignView::copyArtifactPath", "copy path failed", e)
    }
  }, [t])
  const revealArtifact = useCallback(async () => {
    const path = activeArtifactRef.current?.artifactPath
    if (!path) return
    try {
      await tx.openFilePath(path)
    } catch (e) {
      logger.error("design", "DesignView::revealArtifact", "reveal failed", e)
      toast.error(t("design.err.reveal", "打开失败"))
    }
  }, [tx, t])

  // 分享（B7-1）：HTTP/server 模式 = 建只读分享链接（公开 token 快照）+ 复制；
  // 桌面（无公开 server）= 直接导出干净自包含 HTML 供发送（拍板的降级路径）。
  const [sharing, setSharing] = useState(false)
  const [deployOpen, setDeployOpen] = useState(false) // B7-2 CF 部署对话框
  const [inpaintOpen, setInpaintOpen] = useState(false) // 蒙版局部重绘（image 形态）
  // 品牌包形态自选弹窗（默认 落地页+演示+海报，可自选）。
  const [brandPackOpen, setBrandPackOpen] = useState(false)
  const [brandPackKinds, setBrandPackKinds] = useState<Set<ArtifactKind>>(
    () => new Set<ArtifactKind>(["web", "deck", "poster"]),
  )
  // 分享面板（Wave 1-②，仅 server 模式）：点击 toggle，外点关闭。
  const [shareOpen, setShareOpen] = useState(false)
  const shareRef = useRef<HTMLDivElement>(null)
  useClickOutside(
    shareRef,
    useCallback(() => setShareOpen(false), []),
  )
  const handleShare = useCallback(async () => {
    const a = activeArtifactRef.current
    if (!a || sharing) return
    setSharing(true)
    try {
      if (tx.supportsLocalFileOps()) {
        // 桌面：导出干净 HTML（自包含，可直接发送 / 托管）。
        const res = await tx.call<{ filename: string; mime: string; content: string }>(
          "export_design_artifact_cmd",
          { id: a.id, format: "html" },
        )
        if (res?.content) {
          // export_artifact("html") 返回**原始 HTML 字符串**（非 base64）——直接建 blob，
          // 不走 base64ToBlob（其 atob 会在 HTML 字符上抛，review 修复）。
          presentSaveResult(
            await tx.saveFileAs(
              new Blob([res.content], { type: res.mime || "text/html" }),
              res.filename || `${safeFilename(a.title)}.html`,
            ),
            tx,
            t,
            { savedMsg: t("design.share.exported", "已导出可分享的 HTML") },
          )
        }
      } else {
        // server 模式：建/取分享 token → 公开链接（前端由 server 托管故 origin 即公开基址）。
        const res = await tx.call<{ token: string }>("create_design_share_cmd", {
          artifactId: a.id,
        })
        const url = `${window.location.origin}/api/design/share/${res.token}`
        try {
          await navigator.clipboard.writeText(url)
          toast.success(t("design.share.copied", "已复制只读分享链接"))
        } catch {
          toast.success(url) // 剪贴板不可用 → 直接展示链接
        }
      }
    } catch (e) {
      logger.error("design", "DesignView::handleShare", "share failed", e)
      toast.error(t("design.share.failed", "分享失败"))
    } finally {
      setSharing(false)
    }
  }, [tx, t, sharing])

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
        presentSaveResult(
          await tx.saveFileAs(
            new Blob([res.designMd], { type: "text/markdown" }),
            `${safeFilename(name)}-DESIGN.md`,
          ),
          tx,
          t,
        )
      } catch (e) {
        logger.error("design", "DesignView::exportDesignMd", "export failed", e)
        toast.error(t("design.err.export", "导出失败"))
      }
    },
    [tx, t],
  )

  // ── Version history (D1 / B3-3 双栏 live 预览) ────────────────
  // 列表 / 快照预览 / 溯源 / 恢复确认全在 DesignVersionHistoryModal 内；此处只管开关 + 恢复后刷新。
  const [historyOpen, setHistoryOpen] = useState(false)
  const openHistory = useCallback(() => {
    if (!activeArtifact) return
    setHistoryOpen(true)
  }, [activeArtifact])
  const onVersionRestored = useCallback(() => {
    setPreviewKey((k) => k + 1)
    void refreshView() // sync bodyHash/currentVersion so the next visual edit isn't stale
    if (activeProject) void loadArtifacts(activeProject.id)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refreshView, activeProject]) // loadArtifacts/setPreviewKey stable

  // ── 设备视口 (B4-3) + 演示态 (B4-4) ───────────────────────────
  // per-artifact 记忆（localStorage）：切产物时载回上次的设备选择。
  useEffect(() => {
    if (!activeArtifactId) return
    let saved: string | null = null
    try {
      saved = localStorage.getItem(`design:device:${activeArtifactId}`)
    } catch {
      /* localStorage 不可用 */
    }
    setPreviewDevice(
      saved === "desktop" || saved === "tablet" || saved === "mobile" ? saved : "auto",
    )
  }, [activeArtifactId])
  const changeDevice = useCallback(
    (d: PreviewDevice) => {
      setPreviewDevice(d)
      if (!activeArtifactId) return
      try {
        if (d === "auto") localStorage.removeItem(`design:device:${activeArtifactId}`)
        else localStorage.setItem(`design:device:${activeArtifactId}`, d)
      } catch {
        /* localStorage 不可用 → 仅本次会话生效 */
      }
    },
    [activeArtifactId],
  )
  // 测量预览面尺寸（设备模式的适配缩放用）；面随产物条件渲染，故按产物 id 重挂。
  useEffect(() => {
    const el = previewPaneRef.current
    if (!el || typeof ResizeObserver === "undefined") return
    const ro = new ResizeObserver((entries) => {
      const r = entries[0]?.contentRect
      if (r) setPaneSize({ w: r.width, h: r.height })
    })
    ro.observe(el)
    return () => ro.disconnect()
  }, [activeArtifactId])
  // Present（本标签无 chrome）：Escape 退出。
  useEffect(() => {
    if (!presentMode) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setPresentMode(false)
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [presentMode])
  // 演讲者备注：随打开产物解析 metadata 同步。
  useEffect(() => {
    setPresenterNotes(parsePresenterNotes(activeArtifact?.metadata))
  }, [activeArtifact?.id, activeArtifact?.metadata])
  // 演示计时器：进演示归零后每秒 +1。
  useEffect(() => {
    if (!presentMode) {
      setPresentElapsed(0)
      return
    }
    const id = window.setInterval(() => setPresentElapsed((s) => s + 1), 1000)
    return () => window.clearInterval(id)
  }, [presentMode])
  const savePresenterNote = useCallback(
    (slideIndex: number, text: string) => {
      const aid = activeArtifactRef.current?.id
      if (!aid || slideIndex < 0) return
      setPresenterNotes((prev) => {
        const next = [...prev]
        while (next.length <= slideIndex) next.push("")
        next[slideIndex] = text
        void tx
          .call("set_design_presenter_notes_cmd", { artifactId: aid, notes: next })
          .catch((e) =>
            logger.error("design", "DesignView::presenterNote", "save failed", e),
          )
        return next
      })
    },
    [tx],
  )
  const presentFullscreen = useCallback(() => {
    const el = previewPaneRef.current
    if (el?.requestFullscreen) void el.requestFullscreen().catch(() => {})
  }, [])

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
      // 后端带的可操作提示（反爬协作式引导 B1-5 等）优先展示，否则通用文案。
      const msg = e instanceof Error ? e.message.trim() : ""
      toast.error(msg && msg.length <= 300 ? msg : t("design.err.extract", "反向提取失败"))
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
      } else if (deleteTarget.type === "artifacts-batch") {
        // 批量删除（Wave 1-③）：逐个删（后端无批量端点），全部结束后统一刷一次。
        const ids = deleteTarget.ids
        await Promise.all(
          ids.map((id) =>
            tx.call("delete_design_artifact_cmd", { id }).catch((e) => {
              logger.error("design", "DesignView::batchDelete", `delete ${id} failed`, e)
            }),
          ),
        )
        if (activeArtifact && ids.includes(activeArtifact.id)) setActiveArtifact(null)
        if (activeProject) await loadArtifacts(activeProject.id)
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
          // 外部重挂（agent 编辑 / 批注精修）→ 待填钉锚点随 oidmap 重生成而失效，清掉让用户
          // 在新设计上重新点选（review #5）；选中同理失效。
          setPendingPlacement(null)
          setSelected(null)
          // External change (e.g. agent edit) → resync bodyHash/currentVersion so the
          // next visual edit doesn't trip the stale-write guard and get lost.
          if (active && (!p?.artifactId || p.artifactId === active.id)) void refreshView()
        }
        const proj = activeProjectRef.current
        if (proj) void loadArtifacts(proj.id)
      }),
      // ── 流式生成：壳建成 / 逐帧回填 / 定稿 / 失败 ────────────────
      tx.listen("design:artifact_generating", (raw) => {
        const proj = activeProjectRef.current
        if (proj) void loadArtifacts(proj.id)
        // Chat-first flow: the model just spun up a new artifact and nothing is
        // open — auto-focus the generating shell so the stream renders live in
        // the preview instead of the user having to click the new chip.
        const p = parsePayload<{ artifactId?: string }>(raw)
        if (p?.artifactId && !activeArtifactRef.current) {
          void openArtifact({ id: p.artifactId } as DesignArtifact)
        }
      }),
      tx.listen("design:generate_delta", (raw) => {
        const p = parsePayload<{
          artifactId?: string
          streamId?: string
          seq?: number
          css?: string
          bodyHtml?: string
        }>(raw)
        if (!p?.artifactId || !p.streamId) return
        // 只预览当前打开的产物；后台其它产物的流忽略（其磁盘定稿仍会落地）。
        if (p.artifactId !== activeArtifactRef.current?.id) return
        const cur = streamRef.current
        // 新流（首帧 / streamId 变 = failover 重试）→ 重置 seq 基线。
        if (!cur || cur.streamId !== p.streamId || cur.artifactId !== p.artifactId) {
          streamRef.current = { artifactId: p.artifactId, streamId: p.streamId, seq: -1 }
        }
        const seq = typeof p.seq === "number" ? p.seq : 0
        if (seq <= streamRef.current!.seq) return // 丢乱序 / 重复帧
        streamRef.current!.seq = seq
        const css = p.css ?? ""
        const bodyHtml = p.bodyHtml ?? ""
        streamSnapshotRef.current = { artifactId: p.artifactId, css, bodyHtml }
        // CSS 先落（head 已定稿）再灌 body → 无 FOUC。
        postToIframe({ type: "ds_stream_css", css })
        postToIframe({ type: "ds_stream_body", html: bodyHtml })
      }),
      tx.listen("design:generate_done", (raw) => {
        const p = parsePayload<{ artifactId?: string }>(raw)
        const active = activeArtifactRef.current
        if (p?.artifactId && active?.id === p.artifactId) {
          streamRef.current = null
          streamSnapshotRef.current = null
          // 唯一一次受控 swap：刷新视图（status=ready + 新 bodyHash）+ 重挂到定稿 index.html
          // （editable，挂 oid + inspector bridge）。
          void refreshView()
          setPreviewKey((k) => k + 1)
        }
        const proj = activeProjectRef.current
        if (proj) void loadArtifacts(proj.id)
      }),
      tx.listen("design:generate_error", (raw) => {
        const p = parsePayload<{ artifactId?: string }>(raw)
        const active = activeArtifactRef.current
        if (p?.artifactId && active?.id === p.artifactId) {
          streamRef.current = null
          streamSnapshotRef.current = null
          void refreshView() // status=failed + 刷新 bodyHash
          // 后端已把 index.html 降级为干净占位（非 spinner 壳）→ 重挂加载它，避免预览永久转圈。
          setPreviewKey((k) => k + 1)
          // 仅对正在预览的产物提示失败（与 generate_done 对齐）——否则切到别的项目/产物后，
          // 后台产物的失败会给正看着无关视图的用户弹红色误报。
          toast.error(t("design.err.generate", "生成失败，请重试"))
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
  }, [
    tx,
    loadArtifacts,
    loadProjects,
    loadSystems,
    openProject,
    openArtifact,
    refreshView,
    postToIframe,
    t,
  ])

  // ── Preview iframe src ───────────────────────────────────────

  // cache-bust 键 previewKey：生成/编辑/恢复/刷新 index.html 后端 max-age=60，且流式壳与定稿写
  // 同一 index.html——不带 cache-bust 时 remount 会取回缓存的旧页（server 模式尤甚，卡旧内容 ≤60s）。
  const iframeSrc = (() => {
    if (!activeArtifact) return ""
    const base = tx.resolveAssetUrl(`${activeArtifact.artifactPath}/index.html`) ?? ""
    if (!base) return ""
    return `${base}${base.includes("?") ? "&" : "?"}v=${previewKey}`
  })()
  // src 变（换产物 / 内容刷新 / 定稿 swap）→ 进重载态，onLoad 撤（Wave 2-⑥）。字符串相等比较，
  // 流式期不变 src 故不触发（流式走 postMessage，无 spinner 打扰）。
  useEffect(() => {
    if (iframeSrc) setPreviewLoading(true)
  }, [iframeSrc])

  // Preview scaling. "fit" stretches the iframe to fill the pane. A numeric zoom
  // renders at the artifact's natural viewport size and visually scales it, with the
  // wrapper reserving the *scaled* footprint so 100% shows real pixels (not a no-op
  // vs. fit) and 50% shows the whole design at half size with correct scrolling.
  const naturalW = activeArtifact?.viewportW && activeArtifact.viewportW > 0 ? activeArtifact.viewportW : 1024
  const naturalH = activeArtifact?.viewportH && activeArtifact.viewportH > 0 ? activeArtifact.viewportH : 768

  // 设备视口模式（B4-3）：固定逻辑宽高，按测得的预览面尺寸整体缩放适配 + 居中设备框。
  // `auto` 保持原有 zoom 行为（零回归）。
  const devicePreset = previewDevice === "auto" ? null : DEVICE_PRESETS[previewDevice]
  const deviceScale = (() => {
    if (!devicePreset) return 1
    const availW = Math.max(0, paneSize.w - 32)
    const availH = Math.max(0, paneSize.h - 32)
    const sw = devicePreset.w > 0 ? availW / devicePreset.w : 1
    if (devicePreset.h) return Math.min(1, sw, availH / devicePreset.h)
    return Math.min(1, sw) // desktop（无固定高）：只按宽度适配，内容纵向滚
  })()
  const deviceH = devicePreset
    ? devicePreset.h ?? Math.max(400, Math.round((paneSize.h - 32) / (deviceScale || 1)))
    : 0

  // 右键菜单坐标换算用的当前预览缩放（iframe 内 CSS 像素 → 屏上像素）；fit 模式 iframe 100% 无
  // transform 即 1:1。渲染期赋值（同 editModeRef 模式），消息 handler 经 ref 取最新值。
  previewScaleRef.current = devicePreset ? deviceScale : zoom === "fit" ? 1 : zoom
  const scaleStyle: CSSProperties = devicePreset
    ? {
        width: `${devicePreset.w}px`,
        height: `${deviceH}px`,
        border: 0,
        transform: `scale(${deviceScale})`,
        transformOrigin: "top left",
      }
    : zoom === "fit"
      ? { width: "100%", height: "100%", border: 0 }
      : {
          width: `${naturalW}px`,
          height: `${naturalH}px`,
          border: 0,
          transform: `scale(${zoom})`,
          transformOrigin: "top left",
        }
  const frameWrapStyle: CSSProperties | undefined = devicePreset
    ? { width: `${devicePreset.w * deviceScale}px`, height: `${deviceH * deviceScale}px` }
    : zoom === "fit"
      ? undefined
      : { width: `${naturalW * zoom}px`, height: `${naturalH * zoom}px` }
  // B4-1 画框叠层 canvas 尺寸：贴合 iframe **可视 footprint**（纯宽高、无 transform），逐像素与
  // iframe 屏上占位一致。**红线（review 坐标漂移修复）**：不可用 `inset-0`——设备/缩放模式下
  // border-box + 6px 边框会让 content box 比 iframe scaled footprint 窄 12px（footprint 溢出被
  // overflow-hidden 裁），canvas 只覆 content box 而映射用满 clientWidth → 右/下边缘按 12/deviceScale
  // 漂移。改让 canvas 与 iframe 同 footprint（同溢出同裁剪），getBoundingClientRect 一致，映射归零漂移。
  const overlayFrameStyle: CSSProperties = devicePreset
    ? { width: `${devicePreset.w * deviceScale}px`, height: `${deviceH * deviceScale}px` }
    : zoom === "fit"
      ? { width: "100%", height: "100%" }
      : { width: `${naturalW * zoom}px`, height: `${naturalH * zoom}px` }

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
        {activeProject && renamingProject ? (
          <input
            autoFocus
            defaultValue={activeProject.title}
            onBlur={(e) => {
              const v = e.target.value.trim()
              if (v && v !== activeProject.title) void renameProject(activeProject.id, v)
              setRenamingProject(false)
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") (e.target as HTMLInputElement).blur()
              else if (e.key === "Escape") setRenamingProject(false)
            }}
            className="w-48 rounded border border-primary/50 bg-background px-2 py-0.5 text-sm font-semibold outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-1"
          />
        ) : (
          <span
            className={cn(
              "text-sm font-semibold",
              activeProject && "cursor-text rounded px-1 hover:bg-muted",
            )}
            title={activeProject ? t("design.clickRenameProject", "点击改项目名") : undefined}
            onClick={() => {
              if (activeProject) setRenamingProject(true)
            }}
          >
            {activeProject ? activeProject.title : t("design.title", "设计空间")}
          </span>
        )}
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
                  {(() => {
                    // 有活跃产物 → 显示/切换该产物的设计系统（restyle）；否则项目默认系统。
                    const curId = activeArtifact
                      ? activeArtifact.systemId
                      : activeProject.defaultSystemId
                    return (
                      systems.find((s) => s.id === curId)?.name ??
                      t("design.pickSystem", "选择设计系统")
                    )
                  })()}
                </span>
              </Button>
              <DesignSystemPicker
                systems={systems}
                value={
                  (activeArtifact ? activeArtifact.systemId : activeProject.defaultSystemId) ?? null
                }
                onChange={(id) =>
                  activeArtifact ? void restyleActiveArtifact(id) : void setProjectSystem(id)
                }
                open={systemPickerOpen}
                onOpenChange={setSystemPickerOpen}
                onPreviewKit={(id, name) => setKitSystem({ id, name })}
                defaultSystemId={designConfig?.defaultSystemId ?? null}
                onSetDefault={(id) => void setDefaultSystem(id)}
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
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-8 gap-1.5"
                      onClick={() => {
                        setSystemPickerOpen(false)
                        setFigmaImportOpen(true)
                      }}
                    >
                      <Frame className="h-3.5 w-3.5" />
                      {t("design.figma.entry", "从 Figma 导入…")}
                    </Button>
                    {activeProject.defaultSystemId && (
                      <Button
                        variant="ghost"
                        size="sm"
                        className="h-8 gap-1.5"
                        onClick={() => {
                          const sys = systems.find((s) => s.id === activeProject.defaultSystemId)
                          if (!sys) return
                          setSystemPickerOpen(false)
                          setTokenEditorSystem(sys)
                          setTokenEditorOpen(true)
                        }}
                      >
                        <SlidersHorizontal className="h-3.5 w-3.5" />
                        {t("design.editTokens", "编辑设计变量…")}
                      </Button>
                    )}
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
                    {activeProject.defaultSystemId && (
                      <Button
                        variant="ghost"
                        size="sm"
                        className="h-8 gap-1.5"
                        onClick={() => {
                          const sys = systems.find((s) => s.id === activeProject.defaultSystemId)
                          if (!sys) return
                          setSystemPickerOpen(false)
                          setTokenExportSystem(sys)
                          setTokenExportOpen(true)
                        }}
                      >
                        <Braces className="h-3.5 w-3.5" />
                        {t("design.exportTokens", "导出 Token（多平台代码）…")}
                      </Button>
                    )}
                    {activeProject.defaultSystemId && (
                      <Button
                        variant="ghost"
                        size="sm"
                        className="h-8 gap-1.5"
                        onClick={() => {
                          const sys = systems.find((s) => s.id === activeProject.defaultSystemId)
                          if (!sys) return
                          setSystemPickerOpen(false)
                          setCodeBindSystem(sys)
                          setCodeBindOpen(true)
                        }}
                      >
                        <Link2 className="h-3.5 w-3.5" />
                        {t("design.bind.entry", "绑定代码工程…")}
                      </Button>
                    )}
                  </div>
                }
              />
            </>
          )}
          {activeProject && (
            <IconTip label={t("design.pagesOverview", "所有页面 · 文件夹分组")}>
              <Button
                variant={showGrid ? "default" : "ghost"}
                size="icon"
                className="h-8 w-8"
                onClick={() => setShowGrid((v) => !v)}
              >
                <LayoutGrid className="h-4 w-4" />
              </Button>
            </IconTip>
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
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  onSelect={() => {
                    setRefImage(null)
                    setRefExtra("")
                    setRefDialogOpen(true)
                  }}
                >
                  <ImageIcon className="mr-2 h-4 w-4" />
                  {t("design.fromImage", "从参考图生成…")}
                </DropdownMenuItem>
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
          recipes={recipes}
          onPickRecipe={(r) => {
            setHomeKind(r.kind)
            setHomePrompt(r.scenario || r.summary || r.name)
            setHomeRecipeId(r.id)
          }}
          prompt={homePrompt}
          setPrompt={setHomePrompt}
          kind={homeKind}
          setKind={setHomeKind}
          systemId={homeSystemId}
          setSystemId={setHomeSystemId}
          generating={generatingHome}
          onGenerate={() => void generateFromHome()}
          onBrandPack={() => setBrandPackOpen(true)}
          kindLabel={kindLabel}
          onOpen={openProject}
          onDelete={(p) => setDeleteTarget({ type: "project", id: p.id, title: p.title })}
          onRename={renameProject}
          onDuplicate={duplicateProject}
          onBatchDelete={batchDeleteProjects}
          onNewBlank={() => setNewProjectOpen(true)}
        />
      ) : (
        <div className="flex flex-1 min-h-0">
          {/* Left: AI 对话栏（可拖宽 · 可折叠）——设计空间的对话改写主入口 */}
          {chatOpen && (
            <div
              className="flex min-h-0 shrink-0 flex-col border-r"
              style={{ width: chatWidth }}
            >
              <DesignChatPanel
                ref={chatPanelRef}
                projectId={activeProject.id}
                activeArtifact={
                  activeArtifact
                    ? {
                        id: activeArtifact.id,
                        title: activeArtifact.title,
                        kind: activeArtifact.kind,
                      }
                    : null
                }
                systemName={
                  systems.find(
                    (s) =>
                      s.id ===
                      (activeArtifact ? activeArtifact.systemId : activeProject.defaultSystemId),
                  )?.name ?? null
                }
                systemId={
                  (activeArtifact ? activeArtifact.systemId : activeProject.defaultSystemId) ?? null
                }
                onJumpToQuote={(q) => {
                  // 点选带到对话的批注 quote chip → 在预览里聚焦对应元素钉。
                  const m = /^design-comment:(\d+)$/.exec(q.path)
                  if (m) postToIframe({ type: "ds_comment_focus", id: Number(m[1]) })
                }}
                onFocusArtifact={(id) => {
                  // 本轮产物 chip → 打开/聚焦该产物预览（列表里有则直接取，否则按 id 拉全视图）。
                  const found = artifacts.find((a) => a.id === id)
                  void openArtifact(found ?? ({ id } as DesignArtifact))
                }}
                resolveArtifactTitle={(id) => artifacts.find((a) => a.id === id)?.title ?? null}
                recipes={recipes}
                kindLabel={(k) => kindLabel(k as ArtifactKind)}
                active
              />
            </div>
          )}
          {chatOpen && (
            <div
              onPointerDown={startChatResize}
              className="w-1 shrink-0 cursor-col-resize bg-border/40 transition-colors hover:bg-primary/40"
              role="separator"
              aria-orientation="vertical"
            />
          )}

          {/* Right: 顶部产物切换条 + 单产物预览 */}
          <div
            className="relative flex min-w-0 flex-1 flex-col"
            onDragOver={(e) => {
              if (!activeProject) return
              if (Array.from(e.dataTransfer.types).includes("Files")) {
                e.preventDefault()
                if (!dropActive) setDropActive(true)
              }
            }}
            onDragLeave={(e) => {
              // 只在真正离开容器时收起（忽略子元素间冒泡）。
              if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDropActive(false)
            }}
            onDrop={(e) => {
              if (!Array.from(e.dataTransfer.types).includes("Files")) return
              e.preventDefault()
              setDropActive(false)
              void importImageFiles(Array.from(e.dataTransfer.files))
            }}
          >
            {dropActive && (
              <div className="pointer-events-none absolute inset-0 z-30 flex items-center justify-center rounded-lg border-2 border-dashed border-primary/60 bg-primary/5 backdrop-blur-[1px]">
                <div className="flex flex-col items-center gap-1.5 text-primary">
                  <ImageIcon className="h-7 w-7" />
                  <span className="text-sm font-medium">
                    {t("design.dropImport.drop", "松开以导入图片")}
                  </span>
                </div>
              </div>
            )}
            {/* 顶部：对话折叠钮 + 横向产物切换条（原左侧列表收窄成条） */}
            <div className="flex h-11 shrink-0 items-center gap-1.5 overflow-x-auto border-b bg-background/60 px-2">
              <IconTip
                label={chatOpen ? t("design.chat.hide", "隐藏对话") : t("design.chat.show", "显示对话")}
                side="bottom"
              >
                <Button
                  variant="ghost"
                  size="icon"
                  className="h-7 w-7 shrink-0"
                  onClick={() => setChatOpen((v) => !v)}
                >
                  {chatOpen ? (
                    <PanelLeftClose className="h-4 w-4" />
                  ) : (
                    <PanelLeft className="h-4 w-4" />
                  )}
                </Button>
              </IconTip>
              <div className="h-4 w-px shrink-0 bg-border" />
              {loadingArtifacts ? (
                <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
              ) : artifacts.length === 0 ? (
                <span className="px-1 text-xs text-muted-foreground">
                  {t("design.emptyArtifactsInline", "还没有产物——右上角「新建产物」，或直接让左侧 AI 生成。")}
                </span>
              ) : (
                artifacts.map((a) => {
                  const Icon = KIND_ICON[a.kind] ?? Monitor
                  const active = activeArtifact?.id === a.id
                  // 网格开启时改名在网格里进行（避免 chip 与网格卡同时渲染两个 input）。
                  const renaming = renamingArtifactId === a.id && !showGrid
                  return (
                    <div key={a.id} className="group/chip relative shrink-0">
                      {renaming ? (
                        <input
                          autoFocus
                          value={renameDraft}
                          onChange={(e) => setRenameDraft(e.target.value)}
                          onBlur={() => {
                            void renameArtifact(a.id, renameDraft)
                            setRenamingArtifactId(null)
                          }}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") {
                              void renameArtifact(a.id, renameDraft)
                              setRenamingArtifactId(null)
                            } else if (e.key === "Escape") setRenamingArtifactId(null)
                          }}
                          className="w-[150px] rounded-lg border border-primary/50 bg-background px-2.5 py-1 text-xs outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-1"
                        />
                      ) : (
                        <>
                          <button
                            type="button"
                            onClick={() => void openArtifact(a)}
                            onDoubleClick={() => {
                              setRenamingArtifactId(a.id)
                              setRenameDraft(a.title)
                            }}
                            title={t("design.dblClickRename", "双击改名")}
                            className={cn(
                              "flex max-w-[180px] items-center gap-1.5 rounded-lg py-1 pl-2.5 pr-11 text-xs transition-colors",
                              active
                                ? "bg-primary/10 text-primary"
                                : "text-foreground hover:bg-muted",
                            )}
                          >
                            <Icon className="h-3.5 w-3.5 shrink-0 opacity-70" />
                            <span className="truncate">{a.title}</span>
                            {a.status === "generating" && (
                              <Loader2 className="h-3 w-3 shrink-0 animate-spin text-muted-foreground" />
                            )}
                            {a.status === "failed" && (
                              <AlertCircle className="h-3.5 w-3.5 shrink-0 text-destructive" />
                            )}
                            {a.status === "needs_review" && (
                              <ShieldAlert className="h-3.5 w-3.5 shrink-0 text-amber-500" />
                            )}
                          </button>
                          <div className="absolute right-0.5 top-1/2 flex -translate-y-1/2 items-center opacity-0 transition-opacity group-hover/chip:opacity-100">
                            <IconTip label={t("design.duplicatePage", "复制页面")}>
                              <button
                                type="button"
                                onClick={(e) => {
                                  e.stopPropagation()
                                  void duplicateArtifact(a.id)
                                }}
                                className="flex h-5 w-5 items-center justify-center rounded text-muted-foreground hover:text-foreground"
                              >
                                <Copy className="h-3 w-3" />
                              </button>
                            </IconTip>
                            <IconTip label={t("common.delete", "删除")}>
                              <button
                                type="button"
                                onClick={(e) => {
                                  e.stopPropagation()
                                  setDeleteTarget({ type: "artifact", id: a.id, title: a.title })
                                }}
                                className="flex h-5 w-5 items-center justify-center rounded text-muted-foreground hover:text-destructive"
                              >
                                <Trash2 className="h-3 w-3" />
                              </button>
                            </IconTip>
                          </div>
                        </>
                      )}
                    </div>
                  )
                })
              )}
            </div>

            {/* Single-artifact preview */}
            <main className="relative flex flex-1 min-w-0 flex-col bg-muted/30">
            {showGrid && artifactsError && artifacts.length === 0 ? (
              // Wave 2-⑨：库加载失败显式态（区别于空库），带重试。
              <div className="flex flex-1 flex-col items-center justify-center gap-2 text-center">
                <TriangleAlert className="h-6 w-6 text-amber-500" />
                <p className="text-sm text-muted-foreground">
                  {t("design.err.loadLibrary", "产物库加载失败")}
                </p>
                <Button
                  size="sm"
                  variant="outline"
                  className="gap-1"
                  onClick={() => activeProject && void loadArtifacts(activeProject.id)}
                >
                  <RotateCcw className="h-3.5 w-3.5" />
                  {t("common.retry", "重试")}
                </Button>
              </div>
            ) : showGrid && loadingArtifacts && artifacts.length === 0 ? (
              /* 骨架屏：加载中不闪「空库」文案，占位卡对齐真实网格列宽。 */
              <div
                className="grid grid-cols-2 gap-2.5 p-3 sm:grid-cols-3 lg:grid-cols-4"
                aria-busy="true"
                aria-label={t("design.loadingLibrary", "正在加载产物库…")}
              >
                {Array.from({ length: 8 }).map((_, i) => (
                  <div key={i} className="space-y-1.5">
                    <div className="aspect-[4/3] animate-pulse rounded-lg bg-muted/60" />
                    <div className="h-3 w-2/3 animate-pulse rounded bg-muted/50" />
                  </div>
                ))}
              </div>
            ) : showGrid ? (
              /* 页面文件管理面（本轮·源码级复刻 OD DesignFilesPanel）：面包屑 + 文件夹 + 类型分组。 */
              <DesignFilesPanel
                artifacts={artifacts}
                folders={folders}
                activeArtifactId={activeArtifact?.id}
                onOpen={(a) => {
                  void openArtifact(a)
                  setShowGrid(false)
                }}
                onRename={(id, title) => void renameArtifact(id, title)}
                onDuplicate={(id) => void duplicateArtifact(id)}
                onDelete={(a) => setDeleteTarget({ type: "artifact", id: a.id, title: a.title })}
                onMove={(id, folder) => void moveArtifactToFolder(id, folder)}
                onCreateFolder={(path) => void createFolder(path)}
                onRenameFolder={(from, to) => void renameFolder(from, to)}
                onDeleteFolder={(path) => void deleteFolder(path)}
                onReorder={(ids) => void reorderArtifacts(ids)}
                onBatchDelete={(ids) =>
                  setDeleteTarget({
                    type: "artifacts-batch",
                    ids,
                    title: t("design.files.selectedCount", "已选 {{n}} 项", { n: ids.length }),
                  })
                }
                onBatchExport={(ids) => void batchExportArtifacts(ids)}
              />
            ) : activeArtifact ? (
              <>
                {/* 工具栏控件多（编辑模式 + 设备 + 缩放 + 演示 / 刷新 / 评审 / 历史 / 分享 / 导出），
                    窄窗口下必须能**换行**而非溢出裁切：外层 min-h + 允许纵向增高，标题 min-w-0 先截断
                    腾地方，控件组 flex-1 + flex-wrap + justify-end → 右对齐逐行回落、任何宽度都不丢按钮。 */}
                <div className="flex min-h-9 shrink-0 items-center gap-2 border-b bg-background/60 px-3 py-1">
                  <span className="min-w-0 truncate text-xs font-medium text-muted-foreground">
                    {activeArtifact.title}
                  </span>
                  <div className="flex flex-1 flex-wrap items-center justify-end gap-1">
                    {isEditableKind(activeArtifact.kind) && (
                      <>
                        <IconTip
                          label={t("design.editMode", "可视化微调：点选元素改属性")}
                          side="bottom"
                        >
                          <Button
                            variant={editMode ? "default" : "ghost"}
                            size="icon"
                            className="h-6 w-6"
                            onClick={() => {
                              setEditMode((v) => !v)
                              setCommentMode(false)
                              setDrawMode(false)
                            }}
                          >
                            <MousePointerClick className="h-3.5 w-3.5" />
                          </Button>
                        </IconTip>
                        <IconTip
                          label={t("design.comment.mode", "批注：点选元素留反馈")}
                          side="bottom"
                        >
                          <Button
                            variant={commentMode ? "default" : "ghost"}
                            size="icon"
                            className="h-6 w-6"
                            onClick={() => {
                              setCommentMode((v) => !v)
                              setEditMode(false)
                              setDrawMode(false)
                            }}
                          >
                            <MessageSquare className="h-3.5 w-3.5" />
                          </Button>
                        </IconTip>
                        <IconTip
                          label={t("design.draw.mode", "画框批注：框选/画笔标注要改的区域，带截图到对话")}
                          side="bottom"
                        >
                          <Button
                            variant={drawMode ? "default" : "ghost"}
                            size="icon"
                            className="h-6 w-6"
                            onClick={() => {
                              setDrawMode((v) => !v)
                              setEditMode(false)
                              setCommentMode(false)
                            }}
                          >
                            <Highlighter className="h-3.5 w-3.5" />
                          </Button>
                        </IconTip>
                      </>
                    )}
                    {/* 撤销 / 重做可视化编辑（B5，Cmd/Ctrl+Z） */}
                    {(undoStack.length > 0 || redoStack.length > 0) && (
                      <div className="flex items-center rounded-md border border-border/60 p-0.5">
                        <IconTip label={t("design.undo", "撤销")} side="bottom">
                          <button
                            type="button"
                            onClick={undo}
                            disabled={undoStack.length === 0}
                            className="flex h-5 w-6 items-center justify-center rounded text-muted-foreground transition-colors hover:text-foreground disabled:opacity-40"
                          >
                            <Undo2 className="h-3.5 w-3.5" />
                          </button>
                        </IconTip>
                        <IconTip label={t("design.redo", "重做")} side="bottom">
                          <button
                            type="button"
                            onClick={redo}
                            disabled={redoStack.length === 0}
                            className="flex h-5 w-6 items-center justify-center rounded text-muted-foreground transition-colors hover:text-foreground disabled:opacity-40"
                          >
                            <Redo2 className="h-3.5 w-3.5" />
                          </button>
                        </IconTip>
                      </div>
                    )}
                    {/* 设备视口切换（B4-3） */}
                    <div className="flex items-center rounded-md border border-border/60 p-0.5">
                      {(
                        [
                          { id: "auto" as const, label: t("design.deviceAuto", "自动"), icon: null },
                          { id: "desktop" as const, label: t("design.deviceDesktop", "桌面"), icon: Monitor },
                          { id: "tablet" as const, label: t("design.deviceTablet", "平板"), icon: Tablet },
                          { id: "mobile" as const, label: t("design.deviceMobile", "手机"), icon: Smartphone },
                        ] as const
                      ).map((d) => (
                        <IconTip key={d.id} label={d.label} side="bottom">
                          <button
                            type="button"
                            onClick={() => changeDevice(d.id)}
                            className={cn(
                              "flex h-5 items-center justify-center rounded px-1.5 text-[11px] transition-colors",
                              previewDevice === d.id
                                ? "bg-secondary text-foreground"
                                : "text-muted-foreground hover:text-foreground",
                            )}
                          >
                            {d.icon ? <d.icon className="h-3.5 w-3.5" /> : d.label}
                          </button>
                        </IconTip>
                      ))}
                    </div>
                    {/* zoom 仅在自动视口下有意义（设备模式整体缩放适配） */}
                    {previewDevice === "auto" && (
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
                          {[0.5, 0.75, 1, 1.25, 1.5, 2].map((z) => (
                            <SelectItem key={z} value={String(z)}>
                              {Math.round(z * 100)}%
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    )}
                    {/* Deck 页码 + 翻页（Wave 2-⑧）：宿主级，缩放/设备模式下也不被一起缩小。 */}
                    {activeArtifact.kind === "deck" && deckState && deckState.count > 1 && (
                      <div className="flex items-center rounded-md border border-border/60 p-0.5">
                        <IconTip label={t("design.deckPrev", "上一页")} side="bottom">
                          <button
                            type="button"
                            onClick={() => deckNav("ds_slide_prev")}
                            disabled={deckState.active <= 0}
                            className="flex h-5 w-6 items-center justify-center rounded text-muted-foreground transition-colors hover:text-foreground disabled:opacity-40"
                          >
                            <ChevronLeft className="h-3.5 w-3.5" />
                          </button>
                        </IconTip>
                        <span className="px-1 text-[11px] tabular-nums text-muted-foreground">
                          {deckState.active + 1} / {deckState.count}
                        </span>
                        <IconTip label={t("design.deckNext", "下一页")} side="bottom">
                          <button
                            type="button"
                            onClick={() => deckNav("ds_slide_next")}
                            disabled={deckState.active >= deckState.count - 1}
                            className="flex h-5 w-6 items-center justify-center rounded text-muted-foreground transition-colors hover:text-foreground disabled:opacity-40"
                          >
                            <ChevronRight className="h-3.5 w-3.5" />
                          </button>
                        </IconTip>
                      </div>
                    )}
                    {/* Present 演示（B4-4） */}
                    <DropdownMenu>
                      <IconTip label={t("design.present", "演示")} side="bottom">
                        <DropdownMenuTrigger asChild>
                          <Button variant="ghost" size="icon" className="h-6 w-6">
                            <Presentation className="h-3.5 w-3.5" />
                          </Button>
                        </DropdownMenuTrigger>
                      </IconTip>
                      <DropdownMenuContent align="end">
                        <DropdownMenuItem onSelect={() => setPresentMode(true)}>
                          <Presentation className="mr-2 h-4 w-4" />
                          {t("design.presentInTab", "本窗口演示")}
                        </DropdownMenuItem>
                        <DropdownMenuItem onSelect={presentFullscreen}>
                          <Maximize2 className="mr-2 h-4 w-4" />
                          {t("design.presentFullscreen", "全屏演示")}
                        </DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>
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
                    {activeArtifact.kind === "image" && (
                      <IconTip label={t("design.inpaint.button", "蒙版重绘")} side="bottom">
                        <Button
                          variant="ghost"
                          size="icon"
                          className="h-6 w-6"
                          onClick={() => setInpaintOpen(true)}
                        >
                          <Brush className="h-3.5 w-3.5" />
                        </Button>
                      </IconTip>
                    )}
                    {activeArtifact.kind !== "image" && activeArtifact.kind !== "audio" && (
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
                    {activeArtifact.kind !== "image" &&
                      activeArtifact.kind !== "audio" &&
                      activeArtifact.kind !== "component" && (
                        <IconTip label={t("design.pageStyle.button", "页面样式")} side="bottom">
                          <Button
                            variant="ghost"
                            size="icon"
                            className="h-6 w-6"
                            onClick={() => {
                              setPsBackground("")
                              setPsColor("")
                              setPsMaxWidth("")
                              setPageStyleOpen(true)
                            }}
                          >
                            <Paintbrush className="h-3.5 w-3.5" />
                          </Button>
                        </IconTip>
                      )}
                    {activeArtifact.kind !== "image" && activeArtifact.kind !== "audio" && (
                      <IconTip
                        label={
                          parseIsRtl(activeArtifact.metadata)
                            ? t("design.rtl.toLtr", "切换为从左到右")
                            : t("design.rtl.toRtl", "切换为从右到左（RTL）")
                        }
                        side="bottom"
                      >
                        <Button
                          variant="ghost"
                          size="icon"
                          className={cn(
                            "h-6 w-6",
                            parseIsRtl(activeArtifact.metadata) && "text-primary",
                          )}
                          onClick={() => void toggleRtl()}
                        >
                          <span className="text-[13px] font-semibold leading-none">
                            {parseIsRtl(activeArtifact.metadata) ? "‏RTL" : "LTR"}
                          </span>
                        </Button>
                      </IconTip>
                    )}
                    {activeArtifact.kind !== "image" && activeArtifact.kind !== "audio" && (
                      <IconTip label={t("design.review.qualityCheck", "质量审查（可访问性/内容）")} side="bottom">
                        <Button
                          variant="ghost"
                          size="icon"
                          className="h-6 w-6"
                          disabled={reviewing}
                          onClick={() => void runQualityReview()}
                        >
                          {reviewing ? (
                            <Loader2Icon className="h-3.5 w-3.5 animate-spin" />
                          ) : (
                            <ListChecks className="h-3.5 w-3.5" />
                          )}
                        </Button>
                      </IconTip>
                    )}
                    <IconTip label={t("design.history", "版本历史")} side="bottom">
                      <Button variant="ghost" size="icon" className="h-6 w-6" onClick={openHistory}>
                        <History className="h-3.5 w-3.5" />
                      </Button>
                    </IconTip>
                    {tx.supportsLocalFileOps() ? (
                      // 桌面无公网服务器：分享 = 一键导出可分享 HTML（保持一键，不加弹层）。
                      <IconTip label={t("design.share.button", "分享")} side="bottom">
                        <Button
                          variant="ghost"
                          size="icon"
                          className="h-6 w-6"
                          disabled={sharing}
                          onClick={() => void handleShare()}
                        >
                          {sharing ? (
                            <Loader2Icon className="h-3.5 w-3.5 animate-spin" />
                          ) : (
                            <Share2 className="h-3.5 w-3.5" />
                          )}
                        </Button>
                      </IconTip>
                    ) : (
                      // server 模式：分享面板（显示/复制/打开/停止公开链接，Wave 1-②）。
                      <div className="relative" ref={shareRef}>
                        <IconTip label={t("design.share.button", "分享")} side="bottom">
                          <Button
                            variant="ghost"
                            size="icon"
                            className={cn("h-6 w-6", shareOpen && "bg-secondary")}
                            onClick={() => setShareOpen((v) => !v)}
                          >
                            <Share2 className="h-3.5 w-3.5" />
                          </Button>
                        </IconTip>
                        {shareOpen && activeArtifact && (
                          <DesignSharePanel
                            artifactId={activeArtifact.id}
                            origin={window.location.origin}
                          />
                        )}
                      </div>
                    )}
                    <DropdownMenu>
                      <IconTip label={t("design.exportArtifact", "导出本页")} side="bottom">
                        <DropdownMenuTrigger asChild>
                          <Button variant="ghost" size="icon" className="h-6 w-6" disabled={!!exporting}>
                            {exporting ? (
                              <Loader2Icon className="h-3.5 w-3.5 animate-spin" />
                            ) : (
                              <Download className="h-3.5 w-3.5" />
                            )}
                          </Button>
                        </DropdownMenuTrigger>
                      </IconTip>
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
                            {t("design.exportPptx", "PPTX（整页图片）")}
                          </DropdownMenuItem>
                        )}
                        {activeArtifact.kind === "deck" && (
                          <DropdownMenuItem onSelect={() => void handleExport("pptx-outline")}>
                            <FileType2 className="mr-2 h-4 w-4" />
                            {t("design.exportPptxOutline", "PPTX（可编辑文本）")}
                          </DropdownMenuItem>
                        )}
                        {activeArtifact.kind === "motion" && (
                          // 原生强路（浏览器逐帧 + ffmpeg）不依赖 WebCodecs，故 motion 始终提供；
                          // 原生不可用时回退客户端 WebCodecs（若也不支持则导出报错）。
                          <DropdownMenuItem onSelect={() => void handleExport("video")}>
                            <Film className="mr-2 h-4 w-4" />
                            {t("design.exportVideo", "视频 (MP4)")}
                          </DropdownMenuItem>
                        )}
                        <DropdownMenuSeparator />
                        <DropdownMenuItem onSelect={() => void handleExport("zip")}>
                          <FileArchive className="mr-2 h-4 w-4" />
                          {t("design.exportZip", "本页源码 (ZIP)")}
                        </DropdownMenuItem>
                        <DropdownMenuItem onSelect={() => void handleExport("handoff")}>
                          <Braces className="mr-2 h-4 w-4" />
                          {t("design.exportHandoff", "代码交付包 (ZIP)")}
                        </DropdownMenuItem>
                        {tx.supportsLocalFileOps() && activeArtifact.artifactPath && (
                          <>
                            <DropdownMenuSeparator />
                            <DropdownMenuItem onSelect={() => void copyArtifactPath()}>
                              <Link2 className="mr-2 h-4 w-4" />
                              {t("design.copyPath", "复制路径")}
                            </DropdownMenuItem>
                            <DropdownMenuItem onSelect={() => void revealArtifact()}>
                              <FolderOpen className="mr-2 h-4 w-4" />
                              {t("design.revealInFinder", "在文件夹中显示")}
                            </DropdownMenuItem>
                          </>
                        )}
                        <DropdownMenuSeparator />
                        <DropdownMenuItem onSelect={() => setDeployOpen(true)}>
                          <Cloud className="mr-2 h-4 w-4" />
                          {t("design.deploy.menu", "部署到 Cloudflare Pages")}
                        </DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </div>
                </div>
                {activeArtifact.status === "needs_review" && (
                  <div className="flex shrink-0 items-center gap-2 border-b border-amber-400/40 bg-amber-50/70 px-3 py-1.5 text-xs dark:bg-amber-950/25">
                    <ShieldAlert className="h-3.5 w-3.5 shrink-0 text-amber-600 dark:text-amber-400" />
                    <span className="min-w-0 flex-1 truncate text-amber-800 dark:text-amber-200">
                      {parseSelfCheck(activeArtifact.metadata)?.detail ??
                        t("design.review.flagged", "自查发现可能的质量问题，建议复查")}
                    </span>
                    <Button
                      size="sm"
                      className="h-6 shrink-0 gap-1 bg-amber-600 px-2 text-xs text-white hover:bg-amber-700 dark:bg-amber-600 dark:text-white dark:hover:bg-amber-500"
                      onClick={handleFixWithAgent}
                    >
                      <Wand2 className="h-3 w-3" />
                      {t("design.review.fixWithAgent", "让 AI 修复")}
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-6 shrink-0 px-2 text-xs text-amber-800 hover:bg-amber-100 dark:text-amber-200 dark:hover:bg-amber-900/40"
                      onClick={() => void handleReviewArtifact("recheck")}
                    >
                      {t("design.review.recheck", "重新检查")}
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-6 shrink-0 px-2 text-xs text-amber-800 hover:bg-amber-100 dark:text-amber-200 dark:hover:bg-amber-900/40"
                      onClick={() => void handleReviewArtifact("dismiss")}
                    >
                      {t("design.review.dismiss", "标记已复查")}
                    </Button>
                  </div>
                )}
                {(() => {
                  const from = parseDerivedFrom(activeArtifact.metadata)
                  if (!from) return null
                  const target = artifacts.find((a) => a.id === from.id)
                  return (
                    <div className="flex shrink-0 items-center gap-1.5 border-b bg-muted/40 px-3 py-1 text-[11px] text-muted-foreground">
                      <GitFork className="h-3 w-3 shrink-0" />
                      <span className="shrink-0">{t("design.derivedFrom", "派生自")}</span>
                      {target ? (
                        <button
                          type="button"
                          onClick={() => void openArtifact(target)}
                          className="min-w-0 truncate font-medium text-foreground hover:underline"
                        >
                          {from.title}
                        </button>
                      ) : (
                        <span className="min-w-0 truncate font-medium">{from.title}</span>
                      )}
                    </div>
                  )
                })()}
                {activeArtifact.status === "failed" && (
                  <div className="flex shrink-0 items-center gap-2 border-b border-destructive/40 bg-destructive/5 px-3 py-1.5 text-xs">
                    <AlertCircle className="h-3.5 w-3.5 shrink-0 text-destructive" />
                    <span className="min-w-0 flex-1 truncate text-destructive">
                      {t("design.gen.failedBar", "这个页面生成失败了。可在左侧对话里重新描述，或删除重来。")}
                    </span>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-6 shrink-0 px-2 text-xs text-destructive hover:bg-destructive/10"
                      onClick={() =>
                        setDeleteTarget({
                          type: "artifact",
                          id: activeArtifact.id,
                          title: activeArtifact.title,
                        })
                      }
                    >
                      {t("design.gen.deletePage", "删除此页")}
                    </Button>
                  </div>
                )}
                <div
                  ref={previewPaneRef}
                  className={cn(
                    "relative flex-1 overflow-auto p-4",
                    devicePreset && "flex items-center justify-center",
                  )}
                >
                  {editMode && !selected && (
                    <div className="pointer-events-none absolute inset-x-0 top-3 z-10 flex justify-center">
                      <span className="rounded-full bg-primary/90 px-3 py-1 text-xs text-primary-foreground shadow-md">
                        {t("design.editHint", "点选元素改属性，双击文字改文案")}
                      </span>
                    </div>
                  )}
                  <div
                    className={cn(
                      "relative overflow-hidden bg-white",
                      devicePreset
                        ? "shrink-0 rounded-[1.5rem] border-[6px] border-neutral-800 shadow-xl dark:border-neutral-700"
                        : cn(
                            "rounded-lg border shadow-sm",
                            zoom === "fit" ? "mx-auto h-full w-full" : "mx-auto",
                          ),
                      editMode && "ring-2 ring-primary/40",
                      drawMode && "ring-2 ring-primary/40",
                    )}
                    style={frameWrapStyle}
                  >
                    {/* 常驻 iframe（Wave 2-⑥）：**不再按 key 重挂**——内容刷新只改 src 就地导航，
                        旧帧垫底直到新帧首绘，消除 React 卸载重建的白闪。key 仅保留在下方 DrawOverlay
                        （其坐标须随内容重排复位）。滚动保温 + spinner 见 handleIframeLoad / previewLoading。 */}
                    <iframe
                      ref={iframeRef}
                      src={iframeSrc}
                      sandbox="allow-scripts"
                      title={activeArtifact.title}
                      onLoad={handleIframeLoad}
                      className="border-0"
                      style={scaleStyle}
                    />
                    {/* 重载中 spinner 叠层（Wave 2-⑥）：src 变→显示，onLoad→撤。让改稿读作「更新中」。 */}
                    {previewLoading && (
                      <div
                        role="status"
                        aria-live="polite"
                        className="pointer-events-none absolute inset-0 z-10 flex items-center justify-center"
                      >
                        <div className="rounded-full bg-background/70 p-2 shadow-sm backdrop-blur-sm">
                          <Loader2Icon className="h-5 w-5 animate-spin text-muted-foreground" />
                        </div>
                        <span className="sr-only">{t("common.loading", "加载中...")}</span>
                      </div>
                    )}
                    {/* B4-1 画框批注：父层 canvas 叠层（inset-0 = iframe 可视框），工具坞 portal 到未裁剪的
                        pane。仅 drawMode 期条件挂载 —— 卸载即天然复位全部 marks/note，无需 setState 复位。 */}
                    {drawMode && (
                      // key 含 previewKey：内容刷新（agent 编辑 / 精修 / 手动刷新 → iframe 重挂、
                      // 布局可能重排）时叠层随之重挂，天然弃掉旧的归一化 marks，不落到新内容错位处
                      //（review MED：同产物 previewKey 变而叠层不重置会把 v1 marks 合成到 v2 布局）。
                      <DesignDrawOverlay
                        key={`${activeArtifact.id}-${previewKey}`}
                        busy={drawBusy}
                        onExit={() => setDrawMode(false)}
                        onSubmit={handleDrawSubmit}
                        onWheelScroll={forwardScrollToIframe}
                        toolbarHost={previewPaneRef.current}
                        frameStyle={overlayFrameStyle}
                      />
                    )}
                  </div>
                </div>
                {/* Deck 缩略图轨（P0）：整套幻灯片缩略图并排、点选跳页、active 高亮，长 deck 一眼总览 +
                    秒跳任意页。无 JS 的 `#ds-slide-N` + `:target` 纯 CSS 点亮（DeckSlideThumb），复用
                    keep-alive 池。仅纯预览态显示（演示/编辑/批注/画框态让位）。 */}
                {activeArtifact.kind === "deck" &&
                  deckState &&
                  deckState.count > 1 &&
                  iframeSrc &&
                  !presentMode &&
                  !editMode &&
                  !commentMode &&
                  !drawMode && (
                    <div className="flex shrink-0 items-center gap-2 overflow-x-auto border-t bg-muted/30 px-3 py-2">
                      {Array.from({ length: deckState.count }, (_, n) => (
                        <DeckSlideThumb
                          key={n}
                          poolKey={`deck-thumb:${activeArtifact.id}:${n}`}
                          src={`${iframeSrc}#ds-slide-${n}`}
                          index={n}
                          active={n === deckState.active}
                          onSelect={(i) => deckNav("ds_slide_go", i)}
                        />
                      ))}
                    </div>
                  )}
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
          </div>

          {/* Inspector (right) — visual fine-tuning */}
          {editMode && selected && activeArtifact && (
            <DesignInspector
              selected={selected}
              onLiveStyle={handleLiveStyle}
              onCommitStyle={handleCommitStyle}
              onLiveText={handleLiveText}
              onCommitText={handleCommitText}
              onLiveAttr={handleLiveAttr}
              onCommitAttr={handleCommitAttr}
              onPickImage={handlePickImage}
              onDelete={() => void handleDeleteElement()}
              onAddToChat={handleAddSelectedToChat}
              onClose={() => setSelected(null)}
            />
          )}

          {/* 编辑态预览右键菜单（bridge ds_context_menu；非编辑态原生右键不受影响） */}
          {previewCtxMenu && editMode && selected && (
            <div
              className="fixed z-[100] min-w-[168px] rounded-lg border border-border bg-popover p-1 shadow-lg animate-in fade-in-0 zoom-in-95"
              style={{ top: previewCtxMenu.y, left: previewCtxMenu.x }}
              onMouseDown={(e) => e.stopPropagation()}
            >
              <button
                className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-sm text-primary transition-colors hover:bg-primary/10"
                onClick={() => {
                  handleAddSelectedToChat()
                  setPreviewCtxMenu(null)
                }}
              >
                <MessagesSquare className="h-3.5 w-3.5" />
                {t("design.insp.addToChat", "添加到对话")}
              </button>
              <button
                className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-sm text-foreground transition-colors hover:bg-muted/80"
                onClick={() => {
                  handleCtxAddComment()
                  setPreviewCtxMenu(null)
                }}
              >
                <MessageSquare className="h-3.5 w-3.5" />
                {t("design.ctx.comment", "添加批注")}
              </button>
              {!!selected.text?.trim() && (
                <button
                  className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-sm text-foreground transition-colors hover:bg-muted/80"
                  onClick={() => {
                    handleCtxCopyText()
                    setPreviewCtxMenu(null)
                  }}
                >
                  <Copy className="h-3.5 w-3.5" />
                  {t("design.ctx.copyText", "复制文本")}
                </button>
              )}
              <div className="my-1 h-px bg-border" />
              <button
                className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-sm text-destructive transition-colors hover:bg-destructive/10"
                onClick={() => {
                  void handleDeleteElement()
                  setPreviewCtxMenu(null)
                }}
              >
                <Trash2 className="h-3.5 w-3.5" />
                {t("design.insp.deleteEl", "删除元素")}
              </button>
            </div>
          )}

          {/* Comment panel (right) — 批注钉（与 Inspector 互斥） */}
          {commentMode && activeArtifact && (
            <DesignCommentPanel
              comments={comments}
              pending={pendingPlacement}
              onCreate={handleCreateComment}
              onCancelPending={() => setPendingPlacement(null)}
              onResolve={handleResolveComment}
              onEdit={handleEditComment}
              onDelete={handleDeleteComment}
              onFocus={(id) => postToIframe({ type: "ds_comment_focus", id })}
              onSendToChat={handleSendCommentToChat}
              onAddToChat={handleAddCommentToChat}
              onBatchToChat={handleBatchCommentsToChat}
              focusCommentId={focusCommentId}
              onFocusHandled={() => setFocusCommentId(null)}
              onClose={() => setCommentMode(false)}
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
              {promptKind === "audio"
                ? t("design.newAudio", "生成音频")
                : t("design.newImage", "生成图像")}
            </DialogTitle>
          </DialogHeader>
          <Textarea
            autoFocus
            value={imagePrompt}
            onChange={(e) => setImagePrompt(e.target.value)}
            rows={3}
            placeholder={
              promptKind === "audio"
                ? t("design.audioPromptPlaceholder", "旁白文本，或音乐/音效描述（可加 [music] / [sfx] 前缀）…")
                : t("design.imagePromptPlaceholder", "描述你想要的图像…")
            }
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

      {/* 从参考图生成匹配产物（vision 描述 → 生成管线） */}
      <Dialog
        open={refDialogOpen}
        onOpenChange={(o) => {
          if (!o && !refGenerating) setRefDialogOpen(false)
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <ImageIcon className="h-4 w-4" />
              {t("design.fromImageTitle", "从参考图生成匹配产物")}
            </DialogTitle>
          </DialogHeader>
          <div className="space-y-3">
            <div className="flex items-center gap-2">
              <span className="text-sm text-muted-foreground">
                {t("design.fromImageKind", "生成形态")}
              </span>
              <Select value={refKind} onValueChange={(v) => setRefKind(v as ArtifactKind)}>
                <SelectTrigger className="h-8 w-40">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {ARTIFACT_KINDS.filter(
                    (k) => !["image", "audio", "component"].includes(k),
                  ).map((k) => (
                    <SelectItem key={k} value={k}>
                      {kindLabel(k)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <label
              className="flex min-h-32 cursor-pointer flex-col items-center justify-center gap-2 rounded-lg border border-dashed p-4 text-sm text-muted-foreground hover:border-primary/50 hover:bg-muted/30"
              onDragOver={(e) => e.preventDefault()}
              onDrop={(e) => {
                e.preventDefault()
                onPickRefImage(e.dataTransfer.files?.[0] ?? null)
              }}
            >
              {refImage ? (
                <img
                  src={refImage.url}
                  alt=""
                  className="max-h-48 max-w-full rounded object-contain"
                />
              ) : (
                <>
                  <ImageIcon className="h-6 w-6 opacity-60" />
                  <span>{t("design.fromImageDrop", "点击或拖入参考设计图")}</span>
                </>
              )}
              <input
                type="file"
                accept="image/*"
                className="hidden"
                onChange={(e) => onPickRefImage(e.target.files?.[0] ?? null)}
              />
            </label>
            <Textarea
              value={refExtra}
              onChange={(e) => setRefExtra(e.target.value)}
              rows={2}
              placeholder={t(
                "design.fromImageExtra",
                "额外要求（可选）：如「文案改成中文」「用我的品牌色」…",
              )}
              className="resize-none"
            />
          </div>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setRefDialogOpen(false)} disabled={refGenerating}>
              {t("common.cancel", "取消")}
            </Button>
            <Button
              onClick={() => void createFromReferenceImage()}
              disabled={refGenerating || !refImage}
            >
              {refGenerating && <Loader2Icon className="mr-2 h-4 w-4 animate-spin" />}
              {t("design.generate", "生成")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* 设计变量可视化编辑器（P2） */}
      <DesignTokenEditor
        system={tokenEditorSystem}
        open={tokenEditorOpen}
        onOpenChange={setTokenEditorOpen}
        onSaved={(systemId) => {
          void loadSystems()
          // fork 出新系统（内置只读）→ 设为项目默认；就地更新 id 不变，无需改。
          if (activeProjectRef.current && systemId !== activeProjectRef.current.defaultSystemId) {
            void setProjectSystem(systemId)
          }
        }}
      />

      {/* 设计系统套件视图（B1-1，从选择器行内「预览套件」触发） */}
      <DesignKitModal
        systemId={kitSystem?.id ?? null}
        systemName={kitSystem?.name}
        onClose={() => setKitSystem(null)}
      />

      {/* 多平台 Token 导出（P3 工程轴 A） */}
      <DesignTokenExport
        system={tokenExportSystem}
        open={tokenExportOpen}
        onOpenChange={setTokenExportOpen}
      />

      {/* 从 Figma 导入设计系统（P3 工程轴 B） */}
      <DesignFigmaImport
        open={figmaImportOpen}
        onOpenChange={setFigmaImportOpen}
        onImported={(systemId) => {
          void loadSystems()
          if (activeProjectRef.current) void setProjectSystem(systemId)
        }}
      />

      {/* 绑定代码工程 + 同步 token（P3 工程轴 D） */}
      <DesignCodeBinding
        system={codeBindSystem}
        open={codeBindOpen}
        onOpenChange={setCodeBindOpen}
      />

      {/* 导出强路依赖门（MP4→ffmpeg / PDF·PNG→浏览器引擎）：未就绪让用户主动选，不静默降级。 */}
      <Dialog
        open={!!exportGate}
        onOpenChange={(o) => {
          if (!o && !gateInstalling) setExportGate(null)
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {exportGate?.dep === "ffmpeg"
                ? t("design.dep.ffmpegTitle", "MP4 编码器未就绪")
                : t("design.dep.browserTitle", "浏览器渲染引擎未就绪")}
            </DialogTitle>
          </DialogHeader>
          {exportGate?.status.canAutoInstall ? (
            <div className="space-y-3 text-sm text-muted-foreground">
              <p>
                {exportGate?.dep === "ffmpeg"
                  ? t(
                      "design.dep.ffmpegAutoDesc",
                      "MP4 强路导出需要 ffmpeg 编码器（矢量保真、任意时长）。可一键下载安装（约 40MB，仅首次），或改用较低保真的浏览器编码。",
                    )
                  : t(
                      "design.dep.browserAutoDesc",
                      "PDF/PNG 强路导出（矢量可搜 PDF / 全保真 PNG）需要浏览器渲染引擎。可一键下载内置 Chromium（约 150MB，仅首次），或改用较低保真的客户端栅格化。",
                    )}
              </p>
              {gateInstalling && (
                <div className="space-y-1">
                  <Progress value={gateProgress ?? undefined} />
                  <p className="text-xs">
                    {gateProgress != null
                      ? `${gateProgress}%`
                      : t("design.dep.downloading", "下载中…")}
                  </p>
                </div>
              )}
            </div>
          ) : (
            <div className="space-y-3 text-sm text-muted-foreground">
              <p>
                {exportGate?.dep === "ffmpeg"
                  ? t("design.dep.ffmpegManualDesc", "MP4 强路导出需要 ffmpeg。请安装后重试，或改用较低保真的浏览器编码。")
                  : t("design.dep.browserManualDesc", "PDF/PNG 强路导出需要浏览器引擎。请安装 Chrome / Edge / Brave 后重试，或改用较低保真的客户端栅格化。")}
              </p>
              {exportGate?.dep === "ffmpeg" && (
                <>
                  <pre className="overflow-x-auto rounded bg-muted p-2 text-xs">
                    brew install ffmpeg{"\n"}winget install ffmpeg{"\n"}apt install ffmpeg
                  </pre>
                  <p className="text-xs">
                    {t("design.dep.envHint", "或设置环境变量 HA_FFMPEG_PATH 指向 ffmpeg 二进制。")}
                  </p>
                </>
              )}
            </div>
          )}
          <DialogFooter className="gap-2">
            <Button variant="ghost" onClick={() => setExportGate(null)} disabled={gateInstalling}>
              {t("common.cancel", "取消")}
            </Button>
            <Button
              variant="outline"
              onClick={() => void gateUseClient()}
              disabled={gateInstalling}
            >
              {t("design.dep.useClient", "用较低保真导出")}
            </Button>
            {exportGate?.status.canAutoInstall && (
              <Button onClick={() => void gateDownloadAndRetry()} disabled={gateInstalling}>
                {gateInstalling
                  ? t("design.dep.installing", "安装中…")
                  : t("design.dep.download", "下载并导出")}
              </Button>
            )}
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Version history — 双栏 live 预览 + 溯源 + 恢复确认（B3-3） */}
      <DesignVersionHistoryModal
        open={historyOpen}
        onClose={() => setHistoryOpen(false)}
        artifactId={activeArtifact?.id ?? null}
        currentVersion={activeArtifact?.currentVersion ?? 0}
        onRestored={onVersionRestored}
      />

      <DesignDeployModal
        open={deployOpen}
        onClose={() => setDeployOpen(false)}
        artifactId={activeArtifact?.id ?? null}
      />

      <DesignInpaintModal
        open={inpaintOpen}
        onClose={() => setInpaintOpen(false)}
        artifactId={activeArtifact?.id ?? null}
        indexUrl={iframeSrc}
        onDone={() => setPreviewKey((k) => k + 1)}
      />

      {/* 品牌包形态自选 */}
      <Dialog open={brandPackOpen} onOpenChange={setBrandPackOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <Layers className="h-4 w-4" />
              {t("design.brandPack.pickTitle", "生成品牌包")}
            </DialogTitle>
          </DialogHeader>
          <p className="text-xs text-muted-foreground">
            {t(
              "design.brandPack.pickHint",
              "选择要一次生成的形态，它们共用同一设计系统、视觉语言一致（最多 6 个）。",
            )}
          </p>
          <div className="flex flex-wrap gap-2">
            {BRAND_PACK_KINDS.map((k) => {
              const Icon = KIND_ICON[k] ?? Monitor
              const active = brandPackKinds.has(k)
              return (
                <button
                  key={k}
                  type="button"
                  onClick={() =>
                    setBrandPackKinds((prev) => {
                      const next = new Set(prev)
                      if (next.has(k)) next.delete(k)
                      else if (next.size < 6) next.add(k)
                      return next
                    })
                  }
                  className={cn(
                    "flex items-center gap-1.5 rounded-full border px-3 py-1.5 text-sm transition-all duration-150",
                    active
                      ? "border-primary/60 bg-primary/10 font-medium text-primary shadow-sm"
                      : "border-border/60 text-muted-foreground hover:border-primary/40 hover:bg-accent hover:text-foreground",
                  )}
                >
                  {active ? <Check className="h-3.5 w-3.5" /> : <Icon className="h-3.5 w-3.5" />}
                  {kindLabel(k)}
                </button>
              )
            })}
          </div>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setBrandPackOpen(false)}>
              {t("common.cancel", "取消")}
            </Button>
            <Button
              disabled={brandPackKinds.size === 0}
              onClick={() => {
                setBrandPackOpen(false)
                void generateBrandPackFromHome([...brandPackKinds])
              }}
            >
              {t("design.brandPack.pickGenerate", "生成 {{count}} 个", { count: brandPackKinds.size })}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* 本窗口无 chrome 演示态（B4-4）：Escape 退出 */}
      {presentMode && activeArtifact && (
        <div className="fixed inset-0 z-[100] flex flex-col bg-neutral-950">
          <div className="absolute right-4 top-4 z-10 flex gap-2">
            {activeArtifact.kind === "deck" && (
              <IconTip
                label={
                  presenterOpen
                    ? t("design.presenter.hide", "隐藏演讲者备注")
                    : t("design.presenter.show", "显示演讲者备注")
                }
                side="bottom"
              >
                <Button
                  variant="secondary"
                  size="icon"
                  className="h-9 w-9 rounded-full opacity-70 shadow-lg transition-opacity hover:opacity-100"
                  onClick={() => setPresenterOpen((v) => !v)}
                >
                  <StickyNote className="h-4 w-4" />
                </Button>
              </IconTip>
            )}
            <IconTip label={t("design.exitPresent", "退出演示 (Esc)")} side="left">
              <Button
                variant="secondary"
                size="icon"
                className="h-9 w-9 rounded-full opacity-70 shadow-lg transition-opacity hover:opacity-100"
                onClick={() => setPresentMode(false)}
              >
                <X className="h-4 w-4" />
              </Button>
            </IconTip>
          </div>
          <iframe
            ref={presentIframeRef}
            key={`present-${activeArtifact.id}-${previewKey}`}
            src={iframeSrc}
            sandbox="allow-scripts"
            title={activeArtifact.title}
            className="min-h-0 w-full flex-1 border-0 bg-white"
            onLoad={() => {
              // 进演示保温（Wave 2-⑧）：deck 从预览的当前页启动，而非被打回第 1 页。
              const s = deckStateRef.current
              if (activeArtifactRef.current?.kind === "deck" && s && s.active > 0) {
                requestAnimationFrame(() =>
                  presentIframeRef.current?.contentWindow?.postMessage(
                    { type: "ds_slide_go", index: s.active },
                    "*",
                  ),
                )
              }
            }}
          />
          {/* 演讲者备注条（deck）：计时器 + 页码 + 当前页备注（可编辑） */}
          {activeArtifact.kind === "deck" && presenterOpen && deckState && (
            <div className="flex shrink-0 items-stretch gap-3 border-t border-white/10 bg-neutral-900 p-3 text-neutral-200">
              <div className="flex w-28 shrink-0 flex-col items-center justify-center gap-1 rounded-md bg-white/5 px-2 py-1">
                <span className="font-mono text-lg tabular-nums">
                  {`${String(Math.floor(presentElapsed / 60)).padStart(2, "0")}:${String(
                    presentElapsed % 60,
                  ).padStart(2, "0")}`}
                </span>
                <span className="text-[11px] text-neutral-400">
                  {t("design.presenter.slide", "第 {{n}}/{{total}} 页", {
                    n: deckState.active + 1,
                    total: deckState.count,
                  })}
                </span>
              </div>
              <textarea
                value={presenterNotes[deckState.active] ?? ""}
                onChange={(e) => savePresenterNote(deckState.active, e.target.value)}
                placeholder={t("design.presenter.notePlaceholder", "本页演讲者备注（自动保存）")}
                className="min-h-[64px] flex-1 resize-none rounded-md border border-white/10 bg-neutral-950 px-3 py-2 text-sm text-neutral-100 outline-none placeholder:text-neutral-500 focus-visible:ring-1 focus-visible:ring-primary/60"
              />
              <div className="flex shrink-0 flex-col justify-center gap-1">
                <Button
                  variant="secondary"
                  size="sm"
                  className="h-8"
                  disabled={deckState.active <= 0}
                  onClick={() => deckNav("ds_slide_prev")}
                >
                  {t("design.deckPrev", "上一页")}
                </Button>
                <Button
                  variant="secondary"
                  size="sm"
                  className="h-8"
                  disabled={deckState.active >= deckState.count - 1}
                  onClick={() => deckNav("ds_slide_next")}
                >
                  {t("design.deckNext", "下一页")}
                </Button>
              </div>
            </div>
          )}
        </div>
      )}

      {/* 页面级样式编辑 */}
      <Dialog open={pageStyleOpen} onOpenChange={setPageStyleOpen}>
        <DialogContent className="max-w-sm">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <Paintbrush className="h-4 w-4" />
              {t("design.pageStyle.title", "页面样式")}
            </DialogTitle>
          </DialogHeader>
          <p className="text-xs text-muted-foreground">
            {t("design.pageStyle.hint", "作用于整页（body）；留空表示不改该项。与逐元素微调互不影响。")}
          </p>
          <div className="space-y-3">
            <div className="flex items-center gap-2">
              <label className="w-20 shrink-0 text-xs font-medium">
                {t("design.pageStyle.background", "背景色")}
              </label>
              <input
                type="color"
                value={psBackground || "#ffffff"}
                onChange={(e) => setPsBackground(e.target.value)}
                className="h-8 w-10 shrink-0 cursor-pointer rounded border"
                aria-label={t("design.pageStyle.background", "背景色")}
              />
              <Input
                value={psBackground}
                onChange={(e) => setPsBackground(e.target.value)}
                placeholder="#ffffff / transparent"
                className="h-8 flex-1 text-xs"
              />
            </div>
            <div className="flex items-center gap-2">
              <label className="w-20 shrink-0 text-xs font-medium">
                {t("design.pageStyle.color", "文字色")}
              </label>
              <input
                type="color"
                value={psColor || "#111111"}
                onChange={(e) => setPsColor(e.target.value)}
                className="h-8 w-10 shrink-0 cursor-pointer rounded border"
                aria-label={t("design.pageStyle.color", "文字色")}
              />
              <Input
                value={psColor}
                onChange={(e) => setPsColor(e.target.value)}
                placeholder="#111111"
                className="h-8 flex-1 text-xs"
              />
            </div>
            <div className="flex items-center gap-2">
              <label className="w-20 shrink-0 text-xs font-medium">
                {t("design.pageStyle.maxWidth", "最大宽度")}
              </label>
              <Input
                value={psMaxWidth}
                onChange={(e) => setPsMaxWidth(e.target.value)}
                placeholder="1200px / 80rem / none"
                className="h-8 flex-1 text-xs"
              />
            </div>
          </div>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setPageStyleOpen(false)}>
              {t("common.cancel", "取消")}
            </Button>
            <Button onClick={() => void savePageStyle()} disabled={psSaving}>
              {psSaving && <Loader2Icon className="mr-1.5 h-4 w-4 animate-spin" />}
              {t("common.apply", "应用")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* 多镜头质量审查结果 */}
      <Dialog open={reviewFindings !== null} onOpenChange={(o) => !o && setReviewFindings(null)}>
        <DialogContent className="max-w-lg">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <ListChecks className="h-4 w-4" />
              {t("design.review.qualityTitle", "质量审查")}
            </DialogTitle>
          </DialogHeader>
          {reviewFindings && reviewFindings.length === 0 ? (
            <div className="flex flex-col items-center gap-2 py-6 text-center text-sm text-muted-foreground">
              <CheckCircle2 className="h-7 w-7 text-emerald-500" />
              {t("design.review.allClear", "未发现可访问性 / 内容 / 语义问题")}
            </div>
          ) : (
            <ul className="max-h-[55vh] space-y-1.5 overflow-y-auto">
              {reviewFindings?.map((f, i) => (
                <li
                  key={i}
                  className="flex items-start gap-2 rounded-lg border bg-muted/30 px-2.5 py-2 text-xs"
                >
                  <span
                    className={cn(
                      "mt-0.5 shrink-0 rounded px-1.5 py-0.5 text-[10px] font-medium uppercase",
                      f.severity === "warn"
                        ? "bg-amber-500/15 text-amber-600 dark:text-amber-400"
                        : "bg-sky-500/15 text-sky-600 dark:text-sky-400",
                    )}
                  >
                    {t(`design.review.lens.${f.lens}`, f.lens)}
                  </span>
                  <span className="min-w-0 flex-1">{f.message}</span>
                </li>
              ))}
            </ul>
          )}
          <DialogFooter>
            <Button variant="ghost" onClick={() => setReviewFindings(null)}>
              {t("common.close", "关闭")}
            </Button>
          </DialogFooter>
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
                : deleteTarget?.type === "artifacts-batch"
                  ? t("design.deleteBatchDesc", "将永久删除选中的这些页面及其全部版本，无法恢复。")
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

/** 首屏 composer 输入框（memo 隔离打字机轮播的高频重渲染，Wave 2-⑩）。 */
const LaunchComposerTextarea = memo(function LaunchComposerTextarea({
  prompt,
  setPrompt,
  onGenerate,
}: {
  prompt: string
  setPrompt: (v: string) => void
  onGenerate: () => void
}) {
  const { t } = useTranslation()
  const scenes = useMemo(
    () => [
      t("design.scene1", "一个 SaaS 产品的定价页，三档套餐，突出中间档"),
      t("design.scene2", "一份融资路演演示，8 页，深色科技风"),
      t("design.scene3", "一个移动 App 的登录 / 注册页，圆角友好风"),
      t("design.scene4", "一张产品发布海报，大标题 + 关键卖点"),
      t("design.scene5", "一个数据看板，KPI 卡片 + 折线趋势"),
    ],
    [t],
  )
  const typed = useTypewriterPlaceholder(scenes, !prompt.trim())
  return (
    <Textarea
      value={prompt}
      onChange={(e) => setPrompt(e.target.value)}
      onKeyDown={(e) => {
        if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
          e.preventDefault()
          onGenerate()
        }
      }}
      placeholder={
        typed ||
        t("design.launchPlaceholder", "描述你想要的设计，例如「一个 SaaS 产品的定价页，三档套餐」…")
      }
      className="min-h-[72px] resize-none border-0 bg-transparent px-2.5 py-1.5 text-base leading-relaxed shadow-none placeholder:text-muted-foreground/60 focus-visible:ring-0"
    />
  )
})

/** 品牌包可选形态（对齐后端 `is_brand_pack_kind`：媒体/组件形态不进批量文案生成）。 */
const BRAND_PACK_KINDS: ArtifactKind[] = [
  "web",
  "mobile",
  "deck",
  "dashboard",
  "poster",
  "document",
  "email",
]

/** 首次运行场景起步卡（零项目时展示，点选预填形态 + 场景 brief，缓解「不知从何开始」）。 */
const SCENARIO_STARTERS: {
  kind: ArtifactKind
  titleKey: string
  titleFallback: string
  promptKey: string
  promptFallback: string
}[] = [
  {
    kind: "web",
    titleKey: "design.starter.webTitle",
    titleFallback: "产品落地页",
    promptKey: "design.starter.webPrompt",
    promptFallback: "为一款 AI 笔记应用做一个现代落地页：英雄区标题、三个核心卖点、定价卡、行动号召。",
  },
  {
    kind: "mobile",
    titleKey: "design.starter.mobileTitle",
    titleFallback: "移动 App 界面",
    promptKey: "design.starter.mobilePrompt",
    promptFallback: "设计一个健身打卡 App 的首页：今日进度环、本周日历、开始训练按钮。",
  },
  {
    kind: "deck",
    titleKey: "design.starter.deckTitle",
    titleFallback: "演示文稿",
    promptKey: "design.starter.deckPrompt",
    promptFallback: "做一份 6 页的创业融资演示：封面、问题、方案、市场、商业模式、团队。",
  },
  {
    kind: "poster",
    titleKey: "design.starter.posterTitle",
    titleFallback: "活动海报",
    promptKey: "design.starter.posterPrompt",
    promptFallback: "设计一张科技沙龙活动海报：主题、时间地点、嘉宾、报名二维码占位。",
  },
]

function LaunchHome({
  projects,
  loading,
  systems,
  recipes,
  onPickRecipe,
  prompt,
  setPrompt,
  kind,
  setKind,
  systemId,
  setSystemId,
  generating,
  onGenerate,
  onBrandPack,
  kindLabel,
  onOpen,
  onDelete,
  onRename,
  onDuplicate,
  onBatchDelete,
  onNewBlank,
}: {
  projects: DesignProject[]
  loading: boolean
  systems: DesignSystemMeta[]
  recipes: DesignRecipe[]
  onPickRecipe: (r: DesignRecipe) => void
  prompt: string
  setPrompt: (v: string) => void
  kind: ArtifactKind
  setKind: (k: ArtifactKind) => void
  systemId: string | null
  setSystemId: (id: string | null) => void
  generating: boolean
  onGenerate: () => void
  onBrandPack: () => void
  kindLabel: (k: ArtifactKind) => string
  onOpen: (p: DesignProject) => void
  onDelete: (p: DesignProject) => void
  onRename: (id: string, title: string) => void
  onDuplicate: (id: string) => void
  onBatchDelete: (ids: string[]) => void
  onNewBlank: () => void
}) {
  const { t } = useTranslation()
  const [pickerOpen, setPickerOpen] = useState(false)
  const systemName = systems.find((s) => s.id === systemId)?.name

  // ── 项目库管理（B3-1）：搜索 / 网格·列表切换 / 多选批量删 / 改名 ──
  const [query, setQuery] = useState("")
  const [view, setView] = useState<"grid" | "list">(() => {
    if (typeof window === "undefined") return "grid"
    return window.localStorage.getItem("design:projects:view") === "list" ? "list" : "grid"
  })
  const setViewPersist = useCallback((v: "grid" | "list") => {
    setView(v)
    try {
      window.localStorage.setItem("design:projects:view", v)
    } catch {
      /* localStorage 不可用 → 仅本次会话生效 */
    }
  }, [])
  const [selectMode, setSelectMode] = useState(false)
  const [selected, setSelected] = useState<Set<string>>(() => new Set())
  const [renameTarget, setRenameTarget] = useState<DesignProject | null>(null)
  const [renameValue, setRenameValue] = useState("")
  const [batchConfirm, setBatchConfirm] = useState(false)

  const filteredProjects = useMemo(() => {
    const q = query.trim().toLowerCase()
    if (!q) return projects
    return projects.filter((p) => p.title.toLowerCase().includes(q))
  }, [projects, query])

  const toggleSelected = useCallback((id: string) => {
    setSelected((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }, [])
  const exitSelectMode = useCallback(() => {
    setSelectMode(false)
    setSelected(new Set())
  }, [])
  const doBatchDelete = useCallback(() => {
    onBatchDelete([...selected])
    setBatchConfirm(false)
    exitSelectMode()
  }, [selected, onBatchDelete, exitSelectMode])
  const openRename = useCallback((p: DesignProject) => {
    setRenameTarget(p)
    setRenameValue(p.title)
  }, [])
  const commitRename = useCallback(() => {
    if (renameTarget) onRename(renameTarget.id, renameValue)
    setRenameTarget(null)
  }, [renameTarget, renameValue, onRename])

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="mx-auto max-w-4xl px-6 pb-14 pt-16">
        {/* Hero */}
        <div className="mb-8 text-center">
          <div className="mb-5 inline-flex items-center gap-2 text-muted-foreground">
            <span className="flex h-7 w-7 items-center justify-center rounded-lg bg-primary/10 ring-1 ring-inset ring-primary/15">
              <Palette className="h-4 w-4 text-primary" />
            </span>
            <span className="text-sm font-medium tracking-wide">{t("design.title", "设计空间")}</span>
          </div>
          <h1 className="font-serif text-4xl font-semibold tracking-tight text-foreground sm:text-[3.25rem] sm:leading-[1.1]">
            {t("design.launchHeading", "你想设计什么？")}
          </h1>
          <p className="mx-auto mt-4 max-w-lg text-[15px] text-muted-foreground">
            {t("design.launchSub", "一句话描述，直接生成可交付的设计——网页 / 演示 / 海报 / 文档 / 动效。")}
          </p>
        </div>

        {/* Prompt card */}
        <div className="rounded-2xl border border-border/60 bg-card p-3 shadow-sm ring-1 ring-transparent transition-all duration-200 focus-within:border-primary/40 focus-within:shadow-lg focus-within:ring-primary/15">
          {/* 打字机轮播占位隔离在 memo 子组件里（Wave 2-⑩ review LOW）：其 ~20fps 状态更新只
              重渲染这一小块，不再拖动整个 LaunchHome 项目网格。 */}
          <LaunchComposerTextarea prompt={prompt} setPrompt={setPrompt} onGenerate={onGenerate} />
          <div className="mt-1 flex items-center justify-between gap-2 border-t border-border/50 px-1 pt-2">
            <Button
              variant="ghost"
              size="sm"
              className="h-8 gap-1.5 rounded-lg text-muted-foreground hover:text-foreground"
              onClick={() => setPickerOpen(true)}
            >
              <Palette className="h-3.5 w-3.5 opacity-80" />
              <span className="max-w-[160px] truncate">
                {systemName ?? t("design.pickSystem", "选择设计系统")}
              </span>
            </Button>
            <div className="flex items-center gap-2">
              <IconTip label={t("design.brandPack.hint", "一次生成落地页 + 演示 + 海报，共用同一设计系统")} side="top">
                <Button
                  size="sm"
                  variant="outline"
                  className="h-9 rounded-lg px-4 font-medium gap-1.5"
                  disabled={!prompt.trim() || generating}
                  onClick={onBrandPack}
                >
                  <Layers className="h-4 w-4" />
                  {t("design.brandPack.button", "品牌包")}
                </Button>
              </IconTip>
              <Button
                size="sm"
                className="h-9 rounded-lg px-5 font-medium gap-1.5"
                disabled={!prompt.trim() || generating}
                onClick={onGenerate}
              >
                {generating && <Loader2 className="h-4 w-4 animate-spin" />}
                {generating ? t("design.generating", "生成中…") : t("design.generate", "生成")}
              </Button>
            </div>
          </div>
        </div>

        {/* Kind chips */}
        <div className="mt-5 flex flex-wrap justify-center gap-2">
          {ARTIFACT_KINDS.map((k) => {
            const Icon = KIND_ICON[k]
            const active = k === kind
            return (
              <button
                key={k}
                type="button"
                onClick={() => setKind(k)}
                className={cn(
                  "flex items-center gap-1.5 rounded-full border px-3 py-1.5 text-sm transition-all duration-150",
                  active
                    ? "border-primary/60 bg-primary/10 font-medium text-primary shadow-sm"
                    : "border-border/60 text-muted-foreground hover:border-primary/40 hover:bg-accent hover:text-foreground",
                )}
              >
                <Icon className="h-3.5 w-3.5" />
                {kindLabel(k)}
              </button>
            )
          })}
        </div>

        {/* Templates（从模板开始：点选 → 填入形态 + 场景 brief，可编辑后生成；换行网格，不横向滚动） */}
        {recipes.length > 0 && (
          <div className="mt-9">
            <p className="mb-3 text-center text-xs font-medium uppercase tracking-wide text-muted-foreground/80">
              {t("design.startFromTemplate", "从模板开始")}
            </p>
            <div className="grid grid-cols-2 gap-2.5 sm:grid-cols-3 lg:grid-cols-4">
              {recipes.slice(0, 8).map((r) => {
                const Icon = KIND_ICON[r.kind] ?? Monitor
                return (
                  <button
                    key={r.id}
                    type="button"
                    onClick={() => onPickRecipe(r)}
                    title={r.summary}
                    className="group flex flex-col gap-1.5 rounded-xl border border-border/60 bg-card p-3.5 text-left transition-all duration-150 hover:-translate-y-0.5 hover:border-primary/40 hover:shadow-md"
                  >
                    <span className="flex h-8 w-8 items-center justify-center rounded-lg bg-muted text-muted-foreground transition-colors group-hover:bg-primary/10 group-hover:text-primary">
                      <Icon className="h-4 w-4" />
                    </span>
                    <span className="truncate text-sm font-medium">{r.name}</span>
                    <span className="line-clamp-2 text-xs leading-snug text-muted-foreground">
                      {r.summary}
                    </span>
                  </button>
                )
              })}
            </div>
          </div>
        )}

        {/* Projects library（B3-1：搜索 / 网格·列表 / 多选批量删 / 改名·复制） */}
        <div className="mt-12">
          <div className="mb-3 flex flex-wrap items-center gap-2">
            <h2 className="text-sm font-semibold text-muted-foreground">
              {t("design.recentProjects", "最近的项目")}
            </h2>
            <div className="ml-auto flex items-center gap-1.5">
              {projects.length > 0 && (
                <>
                  <div className="relative">
                    <Search className="pointer-events-none absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
                    <Input
                      value={query}
                      onChange={(e) => setQuery(e.target.value)}
                      placeholder={t("design.searchProjects", "搜索项目…")}
                      className="h-8 w-40 pl-7 text-xs"
                    />
                  </div>
                  <div className="flex rounded-lg border border-border/60 p-0.5">
                    <IconTip label={t("design.viewGrid", "网格")}>
                      <button
                        type="button"
                        onClick={() => setViewPersist("grid")}
                        className={cn(
                          "flex h-7 w-7 items-center justify-center rounded-md transition-colors",
                          view === "grid"
                            ? "bg-secondary text-foreground"
                            : "text-muted-foreground hover:text-foreground",
                        )}
                      >
                        <LayoutGrid className="h-3.5 w-3.5" />
                      </button>
                    </IconTip>
                    <IconTip label={t("design.viewList", "列表")}>
                      <button
                        type="button"
                        onClick={() => setViewPersist("list")}
                        className={cn(
                          "flex h-7 w-7 items-center justify-center rounded-md transition-colors",
                          view === "list"
                            ? "bg-secondary text-foreground"
                            : "text-muted-foreground hover:text-foreground",
                        )}
                      >
                        <ListIcon className="h-3.5 w-3.5" />
                      </button>
                    </IconTip>
                  </div>
                  <IconTip label={t("design.selectMultiple", "多选")}>
                    <Button
                      variant={selectMode ? "default" : "ghost"}
                      size="icon"
                      className="h-8 w-8"
                      onClick={() => (selectMode ? exitSelectMode() : setSelectMode(true))}
                    >
                      <CheckSquare className="h-3.5 w-3.5" />
                    </Button>
                  </IconTip>
                </>
              )}
              <Button
                variant="ghost"
                size="sm"
                className="h-8 gap-1 text-xs text-muted-foreground"
                onClick={onNewBlank}
              >
                <Plus className="h-3.5 w-3.5" />
                {t("design.newBlankProject", "空白项目")}
              </Button>
            </div>
          </div>

          {selectMode && (
            <div className="mb-3 flex items-center gap-2 rounded-lg border border-border/60 bg-secondary/40 px-3 py-2 text-sm">
              <span className="text-muted-foreground">
                {t("design.selectedCount", "已选 {{count}} 项", { count: selected.size })}
              </span>
              <div className="ml-auto flex items-center gap-1.5">
                <Button variant="ghost" size="sm" className="h-7" onClick={exitSelectMode}>
                  {t("common.cancel", "取消")}
                </Button>
                <Button
                  variant="destructive"
                  size="sm"
                  className="h-7 gap-1.5"
                  disabled={selected.size === 0}
                  onClick={() => setBatchConfirm(true)}
                >
                  <Trash2 className="h-3.5 w-3.5" />
                  {t("design.deleteSelected", "删除所选")}
                </Button>
              </div>
            </div>
          )}

          {loading ? (
            <div className="flex justify-center py-12">
              <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
            </div>
          ) : projects.length === 0 ? (
            <div className="space-y-4 py-6">
              <p className="text-center text-sm text-muted-foreground">
                {t("design.emptyProjectsHint", "还没有项目——在上面描述一个设计，直接开始。")}
              </p>
              <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
                {SCENARIO_STARTERS.map((s) => (
                  <button
                    key={s.kind + s.titleKey}
                    type="button"
                    disabled={generating}
                    onClick={() => {
                      setKind(s.kind)
                      setPrompt(t(s.promptKey, s.promptFallback))
                    }}
                    className="group flex flex-col gap-1 rounded-xl border bg-card p-3 text-left transition-colors hover:border-primary/50 hover:bg-accent/40 disabled:opacity-50"
                  >
                    <span className="text-sm font-medium">{t(s.titleKey, s.titleFallback)}</span>
                    <span className="line-clamp-2 text-xs text-muted-foreground">
                      {t(s.promptKey, s.promptFallback)}
                    </span>
                  </button>
                ))}
              </div>
            </div>
          ) : filteredProjects.length === 0 ? (
            <div className="rounded-xl border border-dashed py-10 text-center text-sm text-muted-foreground">
              {t("design.noMatchProjects", "没有匹配的项目")}
            </div>
          ) : view === "grid" ? (
            <div className="grid grid-cols-2 gap-4 lg:grid-cols-3">
              {filteredProjects.map((p) => {
                const checked = selected.has(p.id)
                return (
                  <div
                    key={p.id}
                    className={cn(
                      "group relative flex flex-col overflow-hidden rounded-xl border bg-card transition-shadow hover:shadow-md",
                      checked && "ring-2 ring-primary",
                    )}
                  >
                    <button
                      type="button"
                      onClick={() => (selectMode ? toggleSelected(p.id) : onOpen(p))}
                      disabled={generating}
                      aria-label={p.title}
                      className={cn(
                        "flex flex-1 flex-col text-left",
                        generating && "pointer-events-none opacity-60",
                      )}
                    >
                      <div
                        className="aspect-[16/10] overflow-hidden"
                        style={p.color ? { background: p.color } : undefined}
                      >
                        <ProjectThumb projectId={p.id} />
                      </div>
                      <div className="p-3 pr-9">
                        <div className="truncate text-sm font-medium">{p.title}</div>
                        <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
                          {t("design.artifactCount", "{{count}} 个产物", {
                            count: p.artifactCount ?? 0,
                          })}
                          {(p.needsReviewCount ?? 0) > 0 && (
                            <span className="inline-flex items-center gap-0.5 rounded-full bg-amber-500/10 px-1.5 py-px text-[10px] font-medium text-amber-600 ring-1 ring-inset ring-amber-500/20 dark:text-amber-400">
                              <ShieldAlert className="h-2.5 w-2.5" />
                              {p.needsReviewCount}
                            </span>
                          )}
                        </div>
                      </div>
                    </button>
                    {selectMode ? (
                      <div
                        className={cn(
                          "absolute left-2 top-2 flex h-5 w-5 items-center justify-center rounded-md border-2 transition-colors",
                          checked
                            ? "border-primary bg-primary text-primary-foreground"
                            : "border-border bg-background/80",
                        )}
                      >
                        {checked && <Check className="h-3 w-3" />}
                      </div>
                    ) : (
                      <DropdownMenu>
                        <DropdownMenuTrigger asChild>
                          <Button
                            variant="ghost"
                            size="icon"
                            aria-label={t("common.more", "更多")}
                            onClick={(e) => e.stopPropagation()}
                            className="absolute bottom-2 right-2 h-7 w-7 text-muted-foreground opacity-0 transition-opacity hover:text-foreground group-hover:opacity-100 data-[state=open]:opacity-100"
                          >
                            <MoreHorizontal className="h-4 w-4" />
                          </Button>
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end">
                          <DropdownMenuItem onClick={() => openRename(p)}>
                            <Pencil className="mr-2 h-3.5 w-3.5" />
                            {t("common.rename", "重命名")}
                          </DropdownMenuItem>
                          <DropdownMenuItem onClick={() => onDuplicate(p.id)}>
                            <Copy className="mr-2 h-3.5 w-3.5" />
                            {t("common.duplicate", "创建副本")}
                          </DropdownMenuItem>
                          <DropdownMenuSeparator />
                          <DropdownMenuItem
                            className="text-destructive focus:text-destructive"
                            onClick={() => onDelete(p)}
                          >
                            <Trash2 className="mr-2 h-3.5 w-3.5" />
                            {t("common.delete", "删除")}
                          </DropdownMenuItem>
                        </DropdownMenuContent>
                      </DropdownMenu>
                    )}
                  </div>
                )
              })}
            </div>
          ) : (
            <div className="flex flex-col gap-1.5">
              {filteredProjects.map((p) => {
                const checked = selected.has(p.id)
                return (
                  <div
                    key={p.id}
                    className={cn(
                      "group flex items-center gap-3 rounded-lg border bg-card px-2.5 py-2 transition-colors hover:bg-secondary/40",
                      checked && "ring-2 ring-primary",
                    )}
                  >
                    {selectMode && (
                      <button
                        type="button"
                        onClick={() => toggleSelected(p.id)}
                        className={cn(
                          "flex h-5 w-5 shrink-0 items-center justify-center rounded-md border-2 transition-colors",
                          checked
                            ? "border-primary bg-primary text-primary-foreground"
                            : "border-border",
                        )}
                      >
                        {checked && <Check className="h-3 w-3" />}
                      </button>
                    )}
                    <button
                      type="button"
                      onClick={() => (selectMode ? toggleSelected(p.id) : onOpen(p))}
                      disabled={generating}
                      className="flex min-w-0 flex-1 items-center gap-3 text-left"
                    >
                      <div
                        className="h-9 w-14 shrink-0 overflow-hidden rounded-md border"
                        style={p.color ? { background: p.color } : undefined}
                      >
                        <ProjectThumb projectId={p.id} />
                      </div>
                      <div className="min-w-0 flex-1">
                        <div className="truncate text-sm font-medium">{p.title}</div>
                        <div className="text-xs text-muted-foreground">
                          {t("design.artifactCount", "{{count}} 个产物", {
                            count: p.artifactCount ?? 0,
                          })}
                        </div>
                      </div>
                      {(p.needsReviewCount ?? 0) > 0 && (
                        <span className="inline-flex items-center gap-0.5 rounded-full bg-amber-500/10 px-1.5 py-0.5 text-[10px] font-medium text-amber-600 ring-1 ring-inset ring-amber-500/20 dark:text-amber-400">
                          <ShieldAlert className="h-2.5 w-2.5" />
                          {p.needsReviewCount}
                        </span>
                      )}
                      <span className="shrink-0 text-xs text-muted-foreground">
                        {new Date(p.updatedAt).toLocaleDateString()}
                      </span>
                    </button>
                    {!selectMode && (
                      <DropdownMenu>
                        <DropdownMenuTrigger asChild>
                          <Button
                            variant="ghost"
                            size="icon"
                            aria-label={t("common.more", "更多")}
                            className="h-7 w-7 shrink-0 text-muted-foreground opacity-0 transition-opacity hover:text-foreground group-hover:opacity-100 data-[state=open]:opacity-100"
                          >
                            <MoreHorizontal className="h-4 w-4" />
                          </Button>
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end">
                          <DropdownMenuItem onClick={() => openRename(p)}>
                            <Pencil className="mr-2 h-3.5 w-3.5" />
                            {t("common.rename", "重命名")}
                          </DropdownMenuItem>
                          <DropdownMenuItem onClick={() => onDuplicate(p.id)}>
                            <Copy className="mr-2 h-3.5 w-3.5" />
                            {t("common.duplicate", "创建副本")}
                          </DropdownMenuItem>
                          <DropdownMenuSeparator />
                          <DropdownMenuItem
                            className="text-destructive focus:text-destructive"
                            onClick={() => onDelete(p)}
                          >
                            <Trash2 className="mr-2 h-3.5 w-3.5" />
                            {t("common.delete", "删除")}
                          </DropdownMenuItem>
                        </DropdownMenuContent>
                      </DropdownMenu>
                    )}
                  </div>
                )
              })}
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

      {/* 改名对话框 */}
      <Dialog open={renameTarget != null} onOpenChange={(o) => !o && setRenameTarget(null)}>
        <DialogContent className="max-w-sm">
          <DialogHeader>
            <DialogTitle>{t("design.renameProject", "重命名项目")}</DialogTitle>
          </DialogHeader>
          <Input
            value={renameValue}
            onChange={(e) => setRenameValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault()
                commitRename()
              }
            }}
            autoFocus
            placeholder={t("design.projectTitle", "项目名称")}
          />
          <DialogFooter>
            <Button variant="ghost" onClick={() => setRenameTarget(null)}>
              {t("common.cancel", "取消")}
            </Button>
            <Button onClick={commitRename} disabled={!renameValue.trim()}>
              {t("common.save", "保存")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* 批量删确认 */}
      <AlertDialog open={batchConfirm} onOpenChange={(o) => !o && setBatchConfirm(false)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("design.deleteTitle", "确认删除？")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("design.batchDeleteHint", "将删除选中的 {{count}} 个项目及其全部产物，不可撤销。", {
                count: selected.size,
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel", "取消")}</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={(e) => {
                e.preventDefault()
                doBatchDelete()
              }}
            >
              {t("common.delete", "删除")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
