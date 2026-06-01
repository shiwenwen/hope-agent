import { useState, type ReactNode } from "react"
import { useTranslation } from "react-i18next"
import {
  ChevronRight,
  Download,
  Files,
  FolderOpen,
  GitCompare,
  Globe,
  LayoutDashboard,
  Search,
  X,
  type LucideIcon,
} from "lucide-react"
import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { basename } from "@/lib/path"
import { openExternalUrl } from "@/lib/openExternalUrl"
import type { FileChangeMetadata, Message } from "@/types/chat"
import { FileMimeIcon } from "@/components/chat/message/FileCard"
import TaskProgressPanel from "@/components/chat/tasks/TaskProgressPanel"
import type { TaskProgressSnapshot } from "@/components/chat/tasks/taskProgress"
import { useSessionFileChanges, type SessionFileEntry } from "./useSessionFileChanges"
import { useSessionUrlSources, type SessionUrlSource } from "./useSessionUrlSources"
import type { WorkspaceTaskExecutionState } from "./taskExecutionState"

interface WorkspacePanelProps {
  taskSnapshot: TaskProgressSnapshot | null
  taskExecutionState?: WorkspaceTaskExecutionState
  /** 会话消息 —— 文件 / 来源聚合在面板内部进行,面板未打开时零成本。 */
  messages: Message[]
  /** 改写类文件「查看 diff」→ 右侧 diff 面板。 */
  onOpenDiff: (payload: FileChangeMetadata) => void
  /** 当前会话 id,文件打开 / 下载需要它解析作用域。 */
  sessionId?: string | null
  onClose: () => void
}

function domainOf(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, "")
  } catch {
    return url
  }
}

/** Collapsible card section matching TaskProgressPanel's visual language. */
function WorkspaceSection({
  title,
  count,
  icon: Icon,
  children,
  defaultExpanded = true,
}: {
  title: string
  count: number
  icon: LucideIcon
  children: ReactNode
  defaultExpanded?: boolean
}) {
  const [expanded, setExpanded] = useState(defaultExpanded)
  return (
    <div className="overflow-hidden rounded-2xl border border-border/80 bg-card/95 shadow-sm">
      <button
        type="button"
        aria-expanded={expanded}
        className="flex w-full items-center gap-2 px-3 py-2 text-left transition-colors hover:bg-secondary/45"
        onClick={() => setExpanded((v) => !v)}
      >
        <Icon className="h-4 w-4 shrink-0 text-blue-500" />
        <span className="min-w-0 flex-1 truncate text-sm font-medium text-foreground">
          {title}
          <span className="px-1.5 font-normal text-muted-foreground">·</span>
          <span className="font-normal text-muted-foreground tabular-nums">{count}</span>
        </span>
        <ChevronRight
          className={cn(
            "h-4 w-4 shrink-0 text-muted-foreground transition-transform duration-200",
            expanded && "rotate-90",
          )}
        />
      </button>
      {expanded && <div className="border-t border-border/60 px-2 py-2">{children}</div>}
    </div>
  )
}

/**
 * 文件行 —— 样式与操作对齐消息下挂文件(FileAttachments / FileCard):
 * FileMimeIcon + 文件名,主点击打开文件,右侧 下载 / 在文件夹显示。改写类且有
 * 结构化 diff 的文件额外保留一个「查看 diff」按钮(工作台独有)。
 */
function FileRow({
  entry,
  sessionId,
  onOpenDiff,
}: {
  entry: SessionFileEntry
  sessionId?: string | null
  onOpenDiff: (payload: FileChangeMetadata) => void
}) {
  const { t } = useTranslation()
  const transport = getTransport()
  const canReveal = transport.supportsLocalFileOps()
  const name = basename(entry.path)
  const diff = entry.diff
  const btnClass =
    "p-1 rounded hover:bg-muted text-muted-foreground hover:text-foreground transition-colors"

  const handleOpen = () => {
    transport
      .openFilePath(entry.path, { sessionId })
      .catch((e) => logger.error("chat", "WorkspacePanel::openFile", "Failed to open file", e))
  }
  const handleDownload = () => {
    transport
      .downloadFilePath(entry.path, { sessionId, filename: name })
      .catch((e) => logger.error("chat", "WorkspacePanel::download", "Failed to download file", e))
  }
  const handleReveal = () => {
    transport
      .call("reveal_in_folder", { path: entry.path })
      .catch((e) => logger.error("chat", "WorkspacePanel::reveal", "Failed to reveal in folder", e))
  }

  return (
    <div className="flex items-center gap-2 rounded-md border border-border/50 bg-secondary/30 px-2.5 py-1.5 transition-colors hover:bg-secondary/50">
      <FileMimeIcon mime="" name={name} className="h-4 w-4 shrink-0 text-muted-foreground" />
      <IconTip label={entry.path}>
        <button
          type="button"
          onClick={handleOpen}
          className="flex min-w-0 flex-1 items-center gap-2 text-left transition-colors hover:text-foreground"
        >
          <span className="truncate text-xs font-medium text-foreground/90">{name}</span>
          {diff ? (
            <span className="shrink-0 text-[10px] tabular-nums">
              <span className="text-emerald-600 dark:text-emerald-400">+{diff.linesAdded}</span>{" "}
              <span className="text-rose-600 dark:text-rose-400">-{diff.linesRemoved}</span>
            </span>
          ) : entry.kind === "read" ? (
            <span className="shrink-0 text-[10px] text-muted-foreground/70">
              {t("workspace.action.read")}
            </span>
          ) : null}
        </button>
      </IconTip>
      <div className="flex shrink-0 items-center gap-0.5">
        {diff && (
          <IconTip label={t("diffPanel.openDiff", "查看 diff")}>
            <button type="button" onClick={() => onOpenDiff(diff)} className={btnClass}>
              <GitCompare className="h-3.5 w-3.5" />
            </button>
          </IconTip>
        )}
        <IconTip label={t("localModels.actions.download", { defaultValue: "Download" })}>
          <button type="button" onClick={handleDownload} className={btnClass}>
            <Download className="h-3.5 w-3.5" />
          </button>
        </IconTip>
        {canReveal && (
          <IconTip label={t("chat.revealInFolder")}>
            <button type="button" onClick={handleReveal} className={btnClass}>
              <FolderOpen className="h-3.5 w-3.5" />
            </button>
          </IconTip>
        )}
      </div>
    </div>
  )
}

function SourceRow({ source }: { source: SessionUrlSource }) {
  const { t } = useTranslation()
  return (
    <IconTip label={source.url}>
      <button
        type="button"
        onClick={() => openExternalUrl(source.url)}
        className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left transition-colors hover:bg-secondary/45"
      >
        <Globe className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate text-xs text-foreground/90">{domainOf(source.url)}</span>
        {source.origin === "web_search" && (
          <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-secondary/70 px-1.5 py-0.5 text-[10px] text-muted-foreground">
            <Search className="h-2.5 w-2.5" />
            {t("workspace.sourceFromSearch", "搜索")}
          </span>
        )}
      </button>
    </IconTip>
  )
}

function EmptyHint({ children }: { children: ReactNode }) {
  return <div className="px-2 py-3 text-center text-xs text-muted-foreground/70">{children}</div>
}

/**
 * 右侧「工作台」面板:把本会话的任务进度、碰到的文件、引用来源聚合到一处。
 * 文件 / 来源聚合在面板内部(useSessionFileChanges / useSessionUrlSources)进行,
 * 面板未打开时不挂载、零成本。结构骨架对齐 DiffPanel embedded 模式。
 */
export default function WorkspacePanel({
  taskSnapshot,
  taskExecutionState = "idle",
  messages,
  onOpenDiff,
  sessionId,
  onClose,
}: WorkspacePanelProps) {
  const { t } = useTranslation()
  const files = useSessionFileChanges(messages)
  const urlSources = useSessionUrlSources(messages)

  return (
    <div className="flex h-full min-h-0 w-full flex-col overflow-hidden">
      <div className="flex items-center gap-2 border-b border-border px-3 py-2">
        <LayoutDashboard className="h-4 w-4 shrink-0 text-muted-foreground" />
        <span className="truncate text-sm font-medium">{t("workspace.panelTitle", "工作台")}</span>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="ml-auto h-7 w-7 shrink-0"
          onClick={onClose}
          aria-label={t("common.close", "关闭")}
        >
          <X className="h-4 w-4" />
        </Button>
      </div>

      <div className="flex-1 space-y-2 overflow-auto p-2">
        {/* 进度 — 复用 TaskProgressPanel(自带「任务 · N/M」折叠头)。 */}
        {taskSnapshot && taskSnapshot.total > 0 ? (
          <TaskProgressPanel snapshot={taskSnapshot} variant="card" executionState={taskExecutionState} />
        ) : (
          <WorkspaceSection title={t("workspace.sectionProgress", "进度")} count={0} icon={LayoutDashboard}>
            <EmptyHint>{t("workspace.emptyProgress", "暂无任务")}</EmptyHint>
          </WorkspaceSection>
        )}

        {/* 输出 — 本会话碰到的文件(读 + 改)。 */}
        <WorkspaceSection title={t("workspace.sectionOutput", "输出")} count={files.length} icon={Files}>
          {files.length > 0 ? (
            <div className="space-y-1">
              {files.map((entry) => (
                <FileRow key={entry.path} entry={entry} sessionId={sessionId} onOpenDiff={onOpenDiff} />
              ))}
            </div>
          ) : (
            <EmptyHint>{t("workspace.emptyOutput", "还没有碰到文件")}</EmptyHint>
          )}
        </WorkspaceSection>

        {/* 来源 — web_search 命中 + 正文链接。 */}
        <WorkspaceSection title={t("workspace.sectionSources", "来源")} count={urlSources.length} icon={Globe}>
          {urlSources.length > 0 ? (
            <div className="space-y-0.5">
              {urlSources.map((source) => (
                <SourceRow key={source.url} source={source} />
              ))}
            </div>
          ) : (
            <EmptyHint>{t("workspace.emptySources", "还没有引用来源")}</EmptyHint>
          )}
        </WorkspaceSection>
      </div>
    </div>
  )
}
