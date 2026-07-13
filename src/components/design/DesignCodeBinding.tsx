/**
 * 绑定设计系统到代码工程 + 同步多平台 token（P3 工程轴 D）。
 *
 * owner 平面专属：选一个代码工程目录 + 子目录 + 要写的格式 → 绑定；「同步」把该设计系统的
 * 多平台 token 文件（复用工程轴 A 的 token_export）写进目录。写盘经后端 `resolve_binding_write_dir`
 * canonicalize + 包含校验（防逃逸）；HTTP 侧受 `filesystem.allowRemoteWrites` 门，桌面不受限。
 */

import { useCallback, useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Loader2, FolderOpen, RefreshCw, Unlink } from "lucide-react"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import ServerDirectoryBrowser from "@/components/chat/input/ServerDirectoryBrowser"
import { useDirectoryPicker } from "@/components/chat/input/useDirectoryPicker"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { toast } from "sonner"
import type {
  BindingSyncReport,
  DesignCodeBinding as Binding,
  DesignSystemMeta,
} from "@/types/design"

const ALL_FORMATS = ["css", "scss", "ts", "swift", "android", "dtcg"] as const

interface Props {
  system: DesignSystemMeta | null
  open: boolean
  onOpenChange: (open: boolean) => void
  /** 目标目录预填（项目级代码仓库绑定的生效目录）；用户可改。 */
  initialTargetDir?: string
}

export function DesignCodeBinding({ system, open, onOpenChange, initialTargetDir }: Props) {
  const { t } = useTranslation()
  const tx = getTransport()
  const [bindings, setBindings] = useState<Binding[]>([])
  const [loading, setLoading] = useState(false)
  const [targetDir, setTargetDir] = useState("")
  const [subfolder, setSubfolder] = useState("design-tokens")
  const [formats, setFormats] = useState<Set<string>>(new Set(ALL_FORMATS))
  const [binding, setBinding] = useState(false)
  const [syncingId, setSyncingId] = useState<number | null>(null)

  const load = useCallback(async () => {
    if (!system) return
    setLoading(true)
    try {
      const list = await tx.call<Binding[]>("list_design_code_bindings_cmd", {
        systemId: system.id,
      })
      setBindings(list ?? [])
    } catch (e) {
      logger.error("design", "DesignCodeBinding::load", "list bindings failed", e)
    } finally {
      setLoading(false)
    }
  }, [system, tx])

  // 仅在对话框「打开」的上升沿做一次 load + 预填（review F5-binding）：initialTargetDir
  // 事后异步 resolve 不得重跑此块——否则会重填用户刻意清空的目标目录、把 token 写错地方。
  const prevOpenRef = useRef(false)
  useEffect(() => {
    const justOpened = open && !prevOpenRef.current
    prevOpenRef.current = open
    if (justOpened && system) {
      void load()
      if (initialTargetDir) setTargetDir(initialTargetDir)
    }
    if (!open) {
      setTargetDir("")
      setSubfolder("design-tokens")
      setFormats(new Set(ALL_FORMATS))
    }
  }, [open, system, load, initialTargetDir])

  const {
    pick,
    browserOpen,
    setBrowserOpen,
    handleBrowserSelect,
  } = useDirectoryPicker({
    onPicked: setTargetDir,
    errorTitle: t("design.bind.dirInvalid", "目录无效"),
    loggerSource: "DesignCodeBinding::pickDir",
  })

  const toggleFormat = (f: string) => {
    setFormats((prev) => {
      const next = new Set(prev)
      if (next.has(f)) next.delete(f)
      else next.add(f)
      return next
    })
  }

  const doBind = async () => {
    if (!system || !targetDir.trim() || formats.size === 0) return
    setBinding(true)
    try {
      const b = await tx.call<Binding>("bind_design_code_project_cmd", {
        systemId: system.id,
        targetDir: targetDir.trim(),
        subfolder: subfolder.trim(),
        formats: Array.from(formats),
      })
      // 绑定成功后立即同步一次，token 文件即刻落地。
      try {
        const rep = await tx.call<BindingSyncReport>("sync_design_code_binding_cmd", { id: b.id })
        toast.success(
          t("design.bind.boundSynced", "已绑定并同步 {{n}} 个文件", {
            n: rep?.written?.length ?? 0,
          }),
        )
      } catch (se) {
        logger.error("design", "DesignCodeBinding::bindSync", "sync after bind failed", se)
        toast.warning(t("design.bind.boundNoSync", "已绑定，但同步失败，请稍后手动同步"))
      }
      setTargetDir("")
      await load()
    } catch (e) {
      logger.error("design", "DesignCodeBinding::bind", "bind failed", e)
      toast.error(t("design.bind.err", "绑定失败") + `: ${e instanceof Error ? e.message : e}`)
    } finally {
      setBinding(false)
    }
  }

  const doSync = async (id: number) => {
    setSyncingId(id)
    try {
      const rep = await tx.call<BindingSyncReport>("sync_design_code_binding_cmd", { id })
      toast.success(
        t("design.bind.synced", "已同步 {{n}} 个文件", { n: rep?.written?.length ?? 0 }),
      )
      await load()
    } catch (e) {
      logger.error("design", "DesignCodeBinding::sync", "sync failed", e)
      toast.error(t("design.bind.syncErr", "同步失败") + `: ${e instanceof Error ? e.message : e}`)
    } finally {
      setSyncingId(null)
    }
  }

  const doUnbind = async (id: number) => {
    try {
      await tx.call("unbind_design_code_project_cmd", { id })
      await load()
    } catch (e) {
      logger.error("design", "DesignCodeBinding::unbind", "unbind failed", e)
      toast.error(t("design.bind.unbindErr", "解绑失败"))
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[82vh] max-w-2xl overflow-y-auto">
        <DialogHeader>
          <DialogTitle>
            {t("design.bind.title", "绑定代码工程")}
            {system ? ` · ${system.name}` : ""}
          </DialogTitle>
          <DialogDescription>
            {t(
              "design.bind.desc",
              "把设计变量同步为多平台开发者代码，写进代码工程指定目录。可随时重新同步。",
            )}
          </DialogDescription>
        </DialogHeader>

        {/* 新建绑定 */}
        <div className="space-y-3 rounded-lg border p-3">
          <div className="space-y-1.5">
            <Label>{t("design.bind.targetDir", "代码工程目录")}</Label>
            <div className="flex gap-2">
              <Input
                value={targetDir}
                onChange={(e) => setTargetDir(e.target.value)}
                placeholder={t("design.bind.targetDirPlaceholder", "选择或输入工程根目录")}
                className="flex-1 font-mono text-xs"
              />
              <Button variant="outline" size="sm" className="h-9 gap-1.5" onClick={() => void pick()}>
                <FolderOpen className="h-3.5 w-3.5" />
                {t("design.bind.choose", "选择…")}
              </Button>
            </div>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="bind-sub">{t("design.bind.subfolder", "子目录（相对工程根）")}</Label>
            <Input
              id="bind-sub"
              value={subfolder}
              onChange={(e) => setSubfolder(e.target.value)}
              placeholder="design-tokens"
              className="font-mono text-xs"
            />
          </div>
          <div className="space-y-1.5">
            <Label>{t("design.bind.formats", "写入格式")}</Label>
            <div className="flex flex-wrap gap-1.5">
              {ALL_FORMATS.map((f) => (
                <Button
                  key={f}
                  type="button"
                  variant={formats.has(f) ? "secondary" : "outline"}
                  size="sm"
                  className="h-7 text-xs uppercase"
                  onClick={() => toggleFormat(f)}
                >
                  {f}
                </Button>
              ))}
            </div>
          </div>
          <Button
            className="w-full gap-1.5"
            onClick={() => void doBind()}
            disabled={binding || !targetDir.trim() || formats.size === 0}
          >
            {binding && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
            {t("design.bind.bind", "绑定并同步")}
          </Button>
        </div>

        {/* 已有绑定 */}
        <div className="space-y-2">
          <div className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
            {t("design.bind.existing", "已绑定")}
          </div>
          {loading ? (
            <div className="flex justify-center py-6 text-muted-foreground">
              <Loader2 className="h-4 w-4 animate-spin" />
            </div>
          ) : bindings.length === 0 ? (
            <p className="py-4 text-center text-xs text-muted-foreground">
              {t("design.bind.none", "还没有绑定")}
            </p>
          ) : (
            bindings.map((b) => (
              <div key={b.id} className="flex items-center gap-2 rounded-md border p-2.5">
                <div className="min-w-0 flex-1">
                  <div className="truncate font-mono text-xs" data-ha-title-tip={`${b.targetDir}/${b.subfolder}`}>
                    {b.targetDir}
                    {b.subfolder ? `/${b.subfolder}` : ""}
                  </div>
                  <div className="mt-0.5 text-[11px] text-muted-foreground">
                    {b.formats.join(" · ")}
                    {b.lastSyncedAt
                      ? ` · ${t("design.bind.lastSynced", "上次同步")} ${b.lastSyncedAt.slice(0, 19).replace("T", " ")}`
                      : ` · ${t("design.bind.neverSynced", "未同步")}`}
                  </div>
                </div>
                <Button
                  variant="outline"
                  size="sm"
                  className="h-7 gap-1"
                  onClick={() => void doSync(b.id)}
                  disabled={syncingId === b.id}
                >
                  {syncingId === b.id ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <RefreshCw className="h-3.5 w-3.5" />
                  )}
                  {t("design.bind.sync", "同步")}
                </Button>
                <IconTip label={t("design.bind.unbind", "解绑")}>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-7 w-7 p-0 text-muted-foreground"
                    onClick={() => void doUnbind(b.id)}
                  >
                    <Unlink className="h-3.5 w-3.5" />
                  </Button>
                </IconTip>
              </div>
            ))
          )}
        </div>

        <ServerDirectoryBrowser
          open={browserOpen}
          initialPath={targetDir || null}
          onOpenChange={setBrowserOpen}
          onSelect={handleBrowserSelect}
          allowCreate
        />
      </DialogContent>
    </Dialog>
  )
}
