/**
 * 关联代码仓库（项目级，双源绑定）。
 *
 * owner 平面专属：把设计项目绑到一个代码仓库——「本机目录」（存 canonical 绝对路径）
 * 或「Hope Agent 项目」（目录从其 working_dir 实时派生、随用户改动跟随），二选一互斥。
 * 绑定 = 用户显式授权读取该目录：反向提取 `from=codebase` 的 agent 读根随之扩张、
 * 设计对话会话 working_dir 对齐、「实现到代码」以它为实现仓库。agent `design` 工具
 * 无绑定动作（模型不能自授权，红线见 design-space.md）。
 *
 * 与 `DesignCodeBinding`（设计系统级 token 同步、写出方向）是两个正交概念。
 */

import { useCallback, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { AlertTriangle, FolderGit2, FolderOpen, Loader2, Unlink } from "lucide-react"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import ServerDirectoryBrowser from "@/components/chat/input/ServerDirectoryBrowser"
import { useDirectoryPicker } from "@/components/chat/input/useDirectoryPicker"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { toast } from "sonner"
import type { Project } from "@/types/project"
import type { CodeBindingInfo, DesignProject } from "@/types/design"

interface Props {
  project: DesignProject | null
  open: boolean
  onOpenChange: (open: boolean) => void
  /** 绑定 / 解绑成功后回传最新项目行（父层刷新 activeProject）。 */
  onBound?: (project: DesignProject) => void
}

export function DesignRepoBinding({ project, open, onOpenChange, onBound }: Props) {
  const { t } = useTranslation()
  const tx = getTransport()
  const [info, setInfo] = useState<CodeBindingInfo | null>(null)
  const [loading, setLoading] = useState(false)
  const [mode, setMode] = useState<"dir" | "haProject">("dir")
  const [dir, setDir] = useState("")
  const [haProjects, setHaProjects] = useState<Project[]>([])
  const [haProjectId, setHaProjectId] = useState("")
  const [saving, setSaving] = useState(false)

  const load = useCallback(async () => {
    if (!project) return
    setLoading(true)
    try {
      const [binding, projects] = await Promise.all([
        tx.call<CodeBindingInfo>("get_design_project_code_binding_cmd", {
          projectId: project.id,
        }),
        tx.call<Project[]>("list_projects_cmd", {}).catch(() => [] as Project[]),
      ])
      setInfo(binding)
      setHaProjects((projects ?? []).filter((p) => !p.archived))
      // 预填当前绑定；未绑定保持默认 dir 模式空表单。
      if (binding?.source === "haProject") {
        setMode("haProject")
        setHaProjectId(binding.haProjectId ?? "")
      } else if (binding?.source === "dir") {
        setMode("dir")
        setDir(binding.codeDir ?? "")
      }
    } catch (e) {
      logger.error("design", "DesignRepoBinding::load", "load binding failed", e)
    } finally {
      setLoading(false)
    }
  }, [project, tx])

  useEffect(() => {
    if (open && project) void load()
    if (!open) {
      setInfo(null)
      setMode("dir")
      setDir("")
      setHaProjectId("")
    }
  }, [open, project, load])

  const { pick, browserOpen, setBrowserOpen, handleBrowserSelect } = useDirectoryPicker({
    onPicked: setDir,
    errorTitle: t("design.repoBind.dirInvalid", "目录无效"),
    loggerSource: "DesignRepoBinding::pickDir",
  })

  const save = async (payload: { codeDir?: string; haProjectId?: string }) => {
    if (!project) return
    setSaving(true)
    try {
      const updated = await tx.call<DesignProject>("set_design_project_code_binding_cmd", {
        projectId: project.id,
        ...payload,
      })
      onBound?.(updated)
      await load()
      const bound = payload.codeDir || payload.haProjectId
      toast.success(
        bound
          ? t("design.repoBind.bound", "已关联代码仓库")
          : t("design.repoBind.unbound", "已解除关联"),
      )
      if (bound) onOpenChange(false)
    } catch (e) {
      logger.error("design", "DesignRepoBinding::save", "set binding failed", e)
      toast.error(
        t("design.repoBind.err", "关联失败") + `: ${e instanceof Error ? e.message : e}`,
      )
    } finally {
      setSaving(false)
    }
  }

  const canBind = mode === "dir" ? !!dir.trim() : !!haProjectId
  const hasBinding = !!info?.source

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <FolderGit2 className="h-4 w-4" />
            {t("design.repoBind.title", "关联代码仓库")}
            {project ? ` · ${project.title}` : ""}
          </DialogTitle>
          <DialogDescription>
            {t(
              "design.repoBind.desc",
              "关联后：从代码库提取品牌可直接读取该仓库，设计对话能查看其中代码，「实现到代码」会在该仓库中落地实现。",
            )}
          </DialogDescription>
        </DialogHeader>

        {loading ? (
          <div className="flex justify-center py-8 text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
          </div>
        ) : (
          <div className="space-y-4">
            {/* 当前绑定状态 */}
            {hasBinding && (
              <div className="flex items-center gap-2 rounded-md border p-2.5">
                <div className="min-w-0 flex-1">
                  <div className="truncate font-mono text-xs">
                    {info?.resolvedDir ?? info?.codeDir ?? info?.haProjectId}
                  </div>
                  <div className="mt-0.5 flex items-center gap-1.5 text-[11px] text-muted-foreground">
                    {info?.source === "haProject"
                      ? t("design.repoBind.sourceHaProject", "来源：Hope Agent 项目")
                      : t("design.repoBind.sourceDir", "来源：本机目录")}
                    {info?.stale && (
                      <span className="inline-flex items-center gap-0.5 text-destructive">
                        <AlertTriangle className="h-3 w-3" />
                        {t("design.repoBind.stale", "已失效（目录或项目不存在）")}
                      </span>
                    )}
                  </div>
                </div>
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-7 gap-1 text-muted-foreground"
                  disabled={saving}
                  onClick={() => void save({})}
                >
                  <Unlink className="h-3.5 w-3.5" />
                  {t("design.repoBind.unbind", "解除关联")}
                </Button>
              </div>
            )}

            {/* 绑定源切换 */}
            <div className="flex gap-1.5">
              <Button
                type="button"
                variant={mode === "dir" ? "secondary" : "outline"}
                size="sm"
                className="h-7 text-xs"
                onClick={() => setMode("dir")}
              >
                {t("design.repoBind.modeDir", "本机目录")}
              </Button>
              <Button
                type="button"
                variant={mode === "haProject" ? "secondary" : "outline"}
                size="sm"
                className="h-7 text-xs"
                onClick={() => setMode("haProject")}
              >
                {t("design.repoBind.modeHaProject", "Hope Agent 项目")}
              </Button>
            </div>

            {mode === "dir" ? (
              <div className="space-y-1.5">
                <Label>{t("design.repoBind.dir", "仓库目录")}</Label>
                <div className="flex gap-2">
                  <Input
                    value={dir}
                    onChange={(e) => setDir(e.target.value)}
                    placeholder={t("design.repoBind.dirPlaceholder", "选择或输入代码仓库根目录")}
                    className="flex-1 font-mono text-xs"
                  />
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-9 gap-1.5"
                    onClick={() => void pick()}
                  >
                    <FolderOpen className="h-3.5 w-3.5" />
                    {t("design.repoBind.choose", "选择…")}
                  </Button>
                </div>
              </div>
            ) : (
              <div className="space-y-1.5">
                <Label>{t("design.repoBind.haProject", "Hope Agent 项目")}</Label>
                <Select value={haProjectId} onValueChange={setHaProjectId}>
                  <SelectTrigger className="w-full">
                    <SelectValue
                      placeholder={t("design.repoBind.haProjectPlaceholder", "选择项目…")}
                    />
                  </SelectTrigger>
                  <SelectContent>
                    {haProjects.map((p) => (
                      <SelectItem key={p.id} value={p.id}>
                        {p.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <p className="text-[11px] text-muted-foreground">
                  {t(
                    "design.repoBind.haProjectHint",
                    "目录取自该项目的工作目录，项目工作目录变更会自动跟随。",
                  )}
                </p>
              </div>
            )}

            <Button
              className="w-full gap-1.5"
              disabled={saving || !canBind}
              onClick={() =>
                void save(
                  mode === "dir" ? { codeDir: dir.trim() } : { haProjectId },
                )
              }
            >
              {saving && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
              {t("design.repoBind.bind", "关联")}
            </Button>
          </div>
        )}

        <ServerDirectoryBrowser
          open={browserOpen}
          initialPath={dir || null}
          onOpenChange={setBrowserOpen}
          onSelect={handleBrowserSelect}
        />
      </DialogContent>
    </Dialog>
  )
}
