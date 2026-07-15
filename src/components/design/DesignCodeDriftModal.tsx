import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { GitCompareArrows, Loader2 } from "lucide-react"

import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { DiffPanel } from "@/components/chat/diff-panel/DiffPanel"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { toast } from "sonner"
import type { CodeDriftChanges } from "@/types/design"

interface Props {
  open: boolean
  onClose: () => void
  artifactId: string | null
}

/**
 * 「查看代码变更」对话框：绑定仓库里、由本设计稿实现出的落地文件，相对设计稿实现基线的
 * 逐文件 diff。复用主对话右侧的 `DiffPanel`（embedded 模式）——零新增 diff 渲染代码。
 * 数据来自 `design_code_drift_changes_cmd`（后端 `code_sync::drift_changes`）。
 */
export function DesignCodeDriftModal({ open, onClose, artifactId }: Props) {
  const { t } = useTranslation()
  const [changes, setChanges] = useState<CodeDriftChanges | null>(null)
  const [loading, setLoading] = useState(false)
  const [activeIndex, setActiveIndex] = useState(0)
  const [nonce, setNonce] = useState(0)

  useEffect(() => {
    if (!open || !artifactId) return
    let cancelled = false
    setLoading(true)
    setActiveIndex(0)
    getTransport()
      .call<CodeDriftChanges>("design_code_drift_changes_cmd", { artifactId })
      .then((res) => {
        if (cancelled) return
        setChanges(res)
        setNonce((n) => n + 1)
      })
      .catch((e) => {
        if (cancelled) return
        logger.error("design", "DesignCodeDriftModal", "load drift changes failed", e)
        toast.error(t("design.drift.err", "读取代码变更失败"))
        onClose()
      })
      .finally(() => {
        if (!cancelled) setLoading(false)
      })
    return () => {
      cancelled = true
    }
    // 只依赖 open / artifactId：父组件每次重渲染都新建内联 onClose（且 t 引用可能变），若纳入依赖会
    // 令弹窗打开期间父的任何 setState（如 design:code_drift 事件触发 loadArtifacts）都清理并重跑本
    // effect——弹回第 1 个文件、闪 spinner、重复请求，甚至重拉瞬时失败经 catch 关掉用户正看的弹窗。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, artifactId])

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="flex h-[80vh] max-w-4xl flex-col gap-0 overflow-hidden p-0">
        <DialogHeader className="border-b px-5 py-3">
          <DialogTitle className="flex items-center gap-2 text-sm">
            <GitCompareArrows className="h-4 w-4 text-sky-500" />
            {t("design.drift.modalTitle", "代码变更（设计稿落地后）")}
          </DialogTitle>
        </DialogHeader>
        <div className="relative min-h-0 flex-1">
          {loading ? (
            <div className="flex h-full items-center justify-center text-muted-foreground">
              <Loader2 className="h-5 w-5 animate-spin" />
            </div>
          ) : changes && changes.files.length > 0 ? (
            <DiffPanel
              changes={changes.files}
              activeIndex={activeIndex}
              openNonce={nonce}
              onActiveIndexChange={setActiveIndex}
              onClose={onClose}
              embedded
            />
          ) : (
            <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
              {t("design.drift.checkClean", "落地代码与设计稿一致，无变更")}
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  )
}
