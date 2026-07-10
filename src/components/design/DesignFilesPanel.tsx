/**
 * 设计空间「文件管理面」——源码级复刻 open-design 的 DesignFilesPanel（页面分组组织面）。
 *
 * 忠实点：文件夹 = 路径前缀（无一等实体）；面包屑导航 + 置顶 Folders 区 + 按类型分组 section
 *（Pages/Decks/Documents…）+ 单击开页面。我方增强（OD 建了 API 却没接线，这里接上、并用它的
 * API 设计）：新建文件夹、把页面移到文件夹（拖拽到文件夹/面包屑 或 ⋯ 菜单）、文件夹改名/删除；
 * 行上带真实缩略图（比 OD 纯图标列表更强，不弱化）；文件夹内拖动排序（沿用 position）。
 */
import { useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  Folder,
  FolderPlus,
  ChevronRight,
  Home,
  MoreVertical,
  Copy,
  Trash2,
  Pencil,
  FolderInput,
  Check,
  Download,
  X,
} from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { IconTip } from "@/components/ui/tooltip"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  DropdownMenuSeparator,
} from "@/components/ui/dropdown-menu"
import { cn } from "@/lib/utils"
import { ArtifactThumb } from "./ArtifactThumb"
import type { DesignArtifact, ArtifactKind } from "@/types/design"

interface Props {
  artifacts: DesignArtifact[]
  folders: string[]
  activeArtifactId?: string
  onOpen: (a: DesignArtifact) => void
  onRename: (id: string, title: string) => void
  onDuplicate: (id: string) => void
  onDelete: (a: DesignArtifact) => void
  onMove: (id: string, folder: string) => void
  onCreateFolder: (path: string) => void
  onRenameFolder: (from: string, to: string) => void
  onDeleteFolder: (path: string) => void
  onReorder: (orderedIds: string[]) => void
  /** 批量删除选中产物（Wave 1-③，单次确认在 DesignView 侧）。 */
  onBatchDelete: (ids: string[]) => void
  /** 批量导出选中产物（逐个走已有导出保存出口）。 */
  onBatchExport: (ids: string[]) => void
}

/** kind → 类型分组 section（OD 按文件类型分组的对应；一个项目多是 Pages）。 */
const KIND_SECTIONS: { key: string; kinds: ArtifactKind[]; labelKey: string; fallback: string }[] = [
  { key: "pages", kinds: ["web", "mobile"], labelKey: "design.files.secPages", fallback: "页面" },
  { key: "deck", kinds: ["deck"], labelKey: "design.files.secDecks", fallback: "演示" },
  { key: "dashboard", kinds: ["dashboard"], labelKey: "design.files.secDashboards", fallback: "仪表盘" },
  { key: "poster", kinds: ["poster"], labelKey: "design.files.secPosters", fallback: "海报" },
  { key: "document", kinds: ["document"], labelKey: "design.files.secDocuments", fallback: "文档" },
  { key: "email", kinds: ["email"], labelKey: "design.files.secEmails", fallback: "邮件" },
  { key: "component", kinds: ["component"], labelKey: "design.files.secComponents", fallback: "组件" },
  { key: "image", kinds: ["image"], labelKey: "design.files.secImages", fallback: "图像" },
  { key: "motion", kinds: ["motion"], labelKey: "design.files.secMotion", fallback: "动效" },
  { key: "audio", kinds: ["audio"], labelKey: "design.files.secAudio", fallback: "音频" },
]

export default function DesignFilesPanel({
  artifacts,
  folders,
  activeArtifactId,
  onOpen,
  onRename,
  onDuplicate,
  onDelete,
  onMove,
  onCreateFolder,
  onRenameFolder,
  onDeleteFolder,
  onReorder,
  onBatchDelete,
  onBatchExport,
}: Props) {
  const { t } = useTranslation()
  const [currentDir, setCurrentDir] = useState("")
  // 多选（Wave 1-③，OD 式悬停即勾轻量多选，非重模式）。artifacts 变化时剔除失效 id。
  const [selected, setSelected] = useState<Set<string>>(new Set())
  useEffect(() => {
    setSelected((prev) => {
      if (prev.size === 0) return prev
      const live = new Set(artifacts.map((a) => a.id))
      const next = new Set([...prev].filter((id) => live.has(id)))
      return next.size === prev.size ? prev : next
    })
  }, [artifacts])
  const toggleSelected = (id: string) =>
    setSelected((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  const [renamingId, setRenamingId] = useState<string | null>(null)
  const [renameDraft, setRenameDraft] = useState("")
  const [creatingFolder, setCreatingFolder] = useState(false)
  const [newFolderName, setNewFolderName] = useState("")
  const [renamingFolder, setRenamingFolder] = useState<string | null>(null)
  const [folderDraft, setFolderDraft] = useState("")
  const [dropTarget, setDropTarget] = useState<string | null>(null) // folder path being hovered for move
  const dragIdRef = useRef<string | null>(null)

  const prefix = currentDir ? `${currentDir}/` : ""

  // 当前层的子文件夹（比 currentDir 深一段的所有文件夹路径的下一段）。
  const subfolders = useMemo(() => {
    const set = new Set<string>()
    for (const f of folders) {
      if (currentDir === "" ? true : f.startsWith(prefix)) {
        const rest = currentDir === "" ? f : f.slice(prefix.length)
        const seg = rest.split("/")[0]
        if (seg) set.add(currentDir === "" ? seg : `${currentDir}/${seg}`)
      }
    }
    return Array.from(set).sort((a, b) => a.localeCompare(b))
  }, [folders, currentDir, prefix])

  const folderItemCount = (path: string) =>
    artifacts.filter((a) => (a.folder ?? "") === path || (a.folder ?? "").startsWith(`${path}/`)).length

  // 当前层的文件（folder === currentDir），保持 position 顺序。
  const filesHere = useMemo(
    () => artifacts.filter((a) => (a.folder ?? "") === currentDir),
    [artifacts, currentDir],
  )

  const sections = useMemo(
    () =>
      KIND_SECTIONS.map((s) => ({
        ...s,
        items: filesHere.filter((a) => s.kinds.includes(a.kind)),
      })).filter((s) => s.items.length > 0),
    [filesHere],
  )

  const crumbs = currentDir ? currentDir.split("/") : []

  const commitCreateFolder = () => {
    const name = newFolderName.trim()
    if (name) onCreateFolder(currentDir ? `${currentDir}/${name}` : name)
    setCreatingFolder(false)
    setNewFolderName("")
  }
  const commitRenameFolder = (path: string) => {
    const name = folderDraft.trim()
    const parent = path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : ""
    if (name && name !== path.split("/").pop()) {
      onRenameFolder(path, parent ? `${parent}/${name}` : name)
    }
    setRenamingFolder(null)
  }

  // 文件夹内拖动排序：把当前层文件按新序替回全量 position 顺序（其它层不动）。
  const reorderWithin = (dragId: string, overId: string) => {
    const ids = filesHere.map((a) => a.id)
    const from = ids.indexOf(dragId)
    const to = ids.indexOf(overId)
    if (from < 0 || to < 0 || from === to) return
    const reordered = [...ids]
    const [m] = reordered.splice(from, 1)
    reordered.splice(to, 0, m)
    // 映射回全量：遍历全量，遇当前层文件用 reordered 的下一个 id 替换。
    let k = 0
    const full = artifacts.map((a) =>
      (a.folder ?? "") === currentDir ? reordered[k++] : a.id,
    )
    onReorder(full)
  }

  const startRename = (a: DesignArtifact) => {
    setRenamingId(a.id)
    setRenameDraft(a.title)
  }
  const commitRename = (id: string) => {
    onRename(id, renameDraft)
    setRenamingId(null)
  }

  return (
    <div className="flex flex-1 flex-col overflow-hidden">
      {/* 面包屑 + 新建文件夹 */}
      <div className="flex h-9 shrink-0 items-center gap-1 border-b bg-background/60 px-3 text-xs">
        <button
          type="button"
          onClick={() => setCurrentDir("")}
          onDragOver={(e) => {
            e.preventDefault()
            setDropTarget("")
          }}
          onDragLeave={() => setDropTarget((p) => (p === "" ? null : p))}
          onDrop={() => {
            if (dragIdRef.current) onMove(dragIdRef.current, "")
            dragIdRef.current = null
            setDropTarget(null)
          }}
          className={cn(
            "flex items-center gap-1 rounded px-1 py-0.5 text-muted-foreground hover:bg-muted hover:text-foreground",
            dropTarget === "" && "bg-primary/15 text-primary",
          )}
        >
          <Home className="h-3.5 w-3.5" />
          {t("design.files.root", "全部页面")}
        </button>
        {crumbs.map((seg, i) => {
          const path = crumbs.slice(0, i + 1).join("/")
          const last = i === crumbs.length - 1
          return (
            <span key={path} className="flex items-center gap-1">
              <ChevronRight className="h-3 w-3 text-muted-foreground/50" />
              <button
                type="button"
                disabled={last}
                onClick={() => setCurrentDir(path)}
                onDragOver={(e) => {
                  e.preventDefault()
                  setDropTarget(path)
                }}
                onDragLeave={() => setDropTarget((p) => (p === path ? null : p))}
                onDrop={() => {
                  if (dragIdRef.current) onMove(dragIdRef.current, path)
                  dragIdRef.current = null
                  setDropTarget(null)
                }}
                className={cn(
                  "rounded px-1 py-0.5",
                  last ? "font-medium text-foreground" : "text-muted-foreground hover:bg-muted hover:text-foreground",
                  dropTarget === path && "bg-primary/15 text-primary",
                )}
              >
                {seg}
              </button>
            </span>
          )
        })}
        <div className="ml-auto">
          <IconTip label={t("design.files.newFolder", "新建文件夹")}>
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7"
              onClick={() => {
                setCreatingFolder(true)
                setNewFolderName("")
              }}
            >
              <FolderPlus className="h-4 w-4" />
            </Button>
          </IconTip>
        </div>
      </div>

      {/* 批量操作栏（Wave 1-③）：有选中时出现，一次删/导出多个页面。 */}
      {selected.size > 0 && (
        <div className="flex h-9 shrink-0 items-center gap-2 border-b bg-primary/5 px-3 text-xs">
          <span className="font-medium text-foreground">
            {t("design.files.selectedCount", "已选 {{n}} 项", { n: selected.size })}
          </span>
          <div className="ml-auto flex items-center gap-1">
            <Button
              variant="ghost"
              size="sm"
              className="h-6 gap-1 px-2 text-xs"
              onClick={() => onBatchExport([...selected])}
            >
              <Download className="h-3.5 w-3.5" />
              {t("design.files.batchExport", "导出")}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              className="h-6 gap-1 px-2 text-xs text-destructive hover:bg-destructive/10 hover:text-destructive"
              onClick={() => onBatchDelete([...selected])}
            >
              <Trash2 className="h-3.5 w-3.5" />
              {t("design.files.batchDelete", "删除")}
            </Button>
            <Button
              variant="ghost"
              size="icon"
              className="h-6 w-6"
              onClick={() => setSelected(new Set())}
              aria-label={t("design.files.clearSelection", "取消选择")}
            >
              <X className="h-3.5 w-3.5" />
            </Button>
          </div>
        </div>
      )}

      <div className="flex-1 space-y-4 overflow-y-auto p-3">
        {/* Folders 区（置顶） */}
        {(subfolders.length > 0 || creatingFolder) && (
          <div>
            <div className="mb-1.5 px-1 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
              {t("design.files.sectionFolders", "文件夹")} · {subfolders.length}
            </div>
            <div className="grid grid-cols-[repeat(auto-fill,minmax(160px,1fr))] gap-2">
              {creatingFolder && (
                <div className="flex items-center gap-1.5 rounded-lg border border-primary/50 bg-card px-2.5 py-2">
                  <Folder className="h-4 w-4 shrink-0 text-muted-foreground" />
                  <Input
                    autoFocus
                    value={newFolderName}
                    onChange={(e) => setNewFolderName(e.target.value)}
                    onBlur={commitCreateFolder}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") commitCreateFolder()
                      else if (e.key === "Escape") setCreatingFolder(false)
                    }}
                    placeholder={t("design.files.folderNamePh", "文件夹名")}
                    className="h-6 border-0 px-0 text-xs shadow-none focus-visible:ring-1"
                  />
                </div>
              )}
              {subfolders.map((path) => {
                const name = path.split("/").pop() ?? path
                const renaming = renamingFolder === path
                return (
                  <div
                    key={path}
                    role="button"
                    tabIndex={renaming ? -1 : 0}
                    aria-label={path.split("/").pop() ?? path}
                    onClick={() => !renaming && setCurrentDir(path)}
                    onKeyDown={(e) => {
                      // a11y（Wave 1-⑤）：键盘用户可进子文件夹（此前 div-onClick 键盘完全不可达 =
                      // 导航主路径断链）。Enter/Space 进入，忽略重命名态。
                      // **只处理落在卡片本身的键**（e.target===currentTarget）——否则会吞掉从嵌套
                      // ⋯ 菜单 trigger / 重命名 input 冒泡上来的 Enter/Space，使菜单键盘不可达
                      //（对抗 review 定位的 a11y 回归修复）。
                      if (e.target !== e.currentTarget) return
                      if (!renaming && (e.key === "Enter" || e.key === " ")) {
                        e.preventDefault()
                        setCurrentDir(path)
                      }
                    }}
                    onDragOver={(e) => {
                      e.preventDefault()
                      setDropTarget(path)
                    }}
                    onDragLeave={() => setDropTarget((p) => (p === path ? null : p))}
                    onDrop={() => {
                      if (dragIdRef.current) onMove(dragIdRef.current, path)
                      dragIdRef.current = null
                      setDropTarget(null)
                    }}
                    className={cn(
                      "group/folder flex cursor-pointer items-center gap-1.5 rounded-lg border bg-card px-2.5 py-2 transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary",
                      dropTarget === path && "border-primary bg-primary/10 ring-1 ring-primary",
                    )}
                  >
                    <Folder className="h-4 w-4 shrink-0 text-amber-500" />
                    {renaming ? (
                      <Input
                        autoFocus
                        value={folderDraft}
                        onChange={(e) => setFolderDraft(e.target.value)}
                        onClick={(e) => e.stopPropagation()}
                        onBlur={() => commitRenameFolder(path)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") commitRenameFolder(path)
                          else if (e.key === "Escape") setRenamingFolder(null)
                        }}
                        className="h-6 border-0 px-0 text-xs shadow-none focus-visible:ring-1"
                      />
                    ) : (
                      <>
                        <span className="min-w-0 flex-1 truncate text-xs font-medium">{name}</span>
                        <span className="shrink-0 text-[10px] text-muted-foreground">
                          {folderItemCount(path)}
                        </span>
                        <DropdownMenu>
                          <DropdownMenuTrigger asChild onClick={(e) => e.stopPropagation()}>
                            <button
                              type="button"
                              className="flex h-5 w-5 items-center justify-center rounded text-muted-foreground opacity-0 hover:text-foreground group-hover/folder:opacity-100"
                            >
                              <MoreVertical className="h-3.5 w-3.5" />
                            </button>
                          </DropdownMenuTrigger>
                          <DropdownMenuContent align="end" onClick={(e) => e.stopPropagation()}>
                            <DropdownMenuItem
                              onClick={() => {
                                setRenamingFolder(path)
                                setFolderDraft(name)
                              }}
                            >
                              <Pencil className="mr-2 h-3.5 w-3.5" />
                              {t("common.rename", "重命名")}
                            </DropdownMenuItem>
                            <DropdownMenuItem
                              className="text-destructive focus:text-destructive"
                              onClick={() => onDeleteFolder(path)}
                            >
                              <Trash2 className="mr-2 h-3.5 w-3.5" />
                              {t("design.files.deleteFolder", "删除文件夹（页面移到全部）")}
                            </DropdownMenuItem>
                          </DropdownMenuContent>
                        </DropdownMenu>
                      </>
                    )}
                  </div>
                )
              })}
            </div>
          </div>
        )}

        {/* 类型分组 section */}
        {sections.map((section) => (
          <div key={section.key}>
            <div className="mb-1.5 px-1 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
              {t(section.labelKey, section.fallback)} · {section.items.length}
            </div>
            <div className="grid grid-cols-[repeat(auto-fill,minmax(190px,1fr))] gap-4">
              {section.items.map((a) => {
                const renaming = renamingId === a.id
                return (
                  <div
                    key={a.id}
                    draggable={!renaming}
                    onDragStart={() => {
                      dragIdRef.current = a.id
                    }}
                    onDragEnd={() => {
                      // 拖拽结束（无论落到哪/是否取消）统一清理，避免高亮/引用残留（review LOW）。
                      dragIdRef.current = null
                      setDropTarget(null)
                    }}
                    onDragOver={(e) => e.preventDefault()}
                    onDrop={() => {
                      if (dragIdRef.current && dragIdRef.current !== a.id) {
                        reorderWithin(dragIdRef.current, a.id)
                      }
                      dragIdRef.current = null
                    }}
                    className={cn(
                      "group/card relative flex flex-col overflow-hidden rounded-lg border bg-card shadow-sm transition-shadow hover:shadow-md",
                      activeArtifactId === a.id && "ring-2 ring-primary/40",
                      selected.has(a.id) && "ring-2 ring-primary",
                    )}
                  >
                    {/* 选择框（Wave 1-③）：悬停显现 / 选中常驻；点它只切选中不打开产物。 */}
                    <button
                      type="button"
                      aria-pressed={selected.has(a.id)}
                      aria-label={t("design.files.select", "选择")}
                      onClick={(e) => {
                        e.stopPropagation()
                        toggleSelected(a.id)
                      }}
                      className={cn(
                        "absolute left-1.5 top-1.5 z-10 flex h-5 w-5 items-center justify-center rounded border bg-background/90 shadow-sm transition-opacity",
                        selected.has(a.id)
                          ? "border-primary bg-primary text-primary-foreground opacity-100"
                          : "border-border text-transparent opacity-0 focus-visible:opacity-100 group-hover/card:opacity-100",
                      )}
                    >
                      <Check className="h-3.5 w-3.5" />
                    </button>
                    <button
                      type="button"
                      onClick={() => onOpen(a)}
                      className="relative block aspect-[3/4] w-full overflow-hidden bg-muted"
                    >
                      {a.kind === "component" ? (
                        <div className="flex h-full w-full items-center justify-center bg-gradient-to-br from-muted to-muted/40 text-[10px] text-muted-foreground/60">
                          {a.kind}
                        </div>
                      ) : (
                        <ArtifactThumb artifactId={a.id} />
                      )}
                    </button>
                    <div className="flex items-center gap-1 border-t px-2 py-1.5">
                      {renaming ? (
                        <input
                          autoFocus
                          value={renameDraft}
                          onChange={(e) => setRenameDraft(e.target.value)}
                          onBlur={() => commitRename(a.id)}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") commitRename(a.id)
                            else if (e.key === "Escape") setRenamingId(null)
                          }}
                          className="min-w-0 flex-1 rounded border border-primary/50 bg-background px-1.5 py-0.5 text-xs outline-none focus-visible:ring-2 focus-visible:ring-primary"
                        />
                      ) : (
                        <span
                          className="min-w-0 flex-1 cursor-text truncate text-xs"
                          title={t("design.dblClickRename", "双击改名")}
                          onDoubleClick={() => startRename(a)}
                        >
                          {a.title}
                        </span>
                      )}
                      <DropdownMenu>
                        <DropdownMenuTrigger asChild>
                          <button
                            type="button"
                            className="flex h-5 w-5 shrink-0 items-center justify-center rounded text-muted-foreground opacity-0 hover:text-foreground group-hover/card:opacity-100"
                          >
                            <MoreVertical className="h-3.5 w-3.5" />
                          </button>
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end" className="max-h-80 overflow-y-auto">
                          <DropdownMenuItem onClick={() => startRename(a)}>
                            <Pencil className="mr-2 h-3.5 w-3.5" />
                            {t("common.rename", "重命名")}
                          </DropdownMenuItem>
                          <DropdownMenuItem onClick={() => onDuplicate(a.id)}>
                            <Copy className="mr-2 h-3.5 w-3.5" />
                            {t("design.duplicatePage", "复制页面")}
                          </DropdownMenuItem>
                          <DropdownMenuSeparator />
                          {/* 移到文件夹（扁平；ui/dropdown 无 submenu）。 */}
                          <div className="flex items-center gap-1.5 px-2 py-1 text-[10px] uppercase tracking-wide text-muted-foreground">
                            <FolderInput className="h-3 w-3" />
                            {t("design.files.moveTo", "移到文件夹")}
                          </div>
                          <DropdownMenuItem
                            disabled={(a.folder ?? "") === ""}
                            onClick={() => onMove(a.id, "")}
                          >
                            <Home className="mr-2 h-3.5 w-3.5" />
                            {t("design.files.root", "全部页面")}
                          </DropdownMenuItem>
                          {folders.map((f) => (
                            <DropdownMenuItem
                              key={f}
                              disabled={(a.folder ?? "") === f}
                              onClick={() => onMove(a.id, f)}
                            >
                              <Folder className="mr-2 h-3.5 w-3.5 text-amber-500" />
                              {f}
                            </DropdownMenuItem>
                          ))}
                          <DropdownMenuSeparator />
                          <DropdownMenuItem
                            className="text-destructive focus:text-destructive"
                            onClick={() => onDelete(a)}
                          >
                            <Trash2 className="mr-2 h-3.5 w-3.5" />
                            {t("common.delete", "删除")}
                          </DropdownMenuItem>
                        </DropdownMenuContent>
                      </DropdownMenu>
                    </div>
                  </div>
                )
              })}
            </div>
          </div>
        ))}

        {subfolders.length === 0 && sections.length === 0 && !creatingFolder && (
          <div className="flex h-full flex-col items-center justify-center gap-2 py-16 text-center text-xs text-muted-foreground">
            <Folder className="h-8 w-8 text-muted-foreground/30" />
            {currentDir
              ? t("design.files.emptyFolder", "这个文件夹还没有页面——把页面拖进来，或用 ⋯ 菜单移入。")
              : t("design.emptyArtifactsInline", "还没有产物——右上角「新建产物」，或直接让左侧 AI 生成。")}
          </div>
        )}
      </div>
    </div>
  )
}
