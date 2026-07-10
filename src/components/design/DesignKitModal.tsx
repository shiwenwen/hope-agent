import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Loader2, Palette } from "lucide-react"

import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"

interface Props {
  /** 要预览套件的设计系统 id；null = 关闭。 */
  systemId: string | null
  /** 系统显示名（标题用）。 */
  systemName?: string | null
  onClose: () => void
}

/**
 * 设计系统「套件视图」模态（B1-1）：把系统 tokens 渲染的自包含套件 HTML（后端
 * `get_design_system_kit_cmd`）放进沙箱 iframe 预览——色板 / 字阶 / 间距 / 圆角+阴影 /
 * 组件 showcase，全走 `var(--ds-*)`，套件即系统真实视觉，内含明/暗表面切换。
 * 浏览器零编译零网络（`sandbox="allow-scripts"`，opaque origin）。
 */
export function DesignKitModal({ systemId, systemName, onClose }: Props) {
  const { t } = useTranslation()
  const [html, setHtml] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)

  // 渲染期重置：systemId 变了立即清旧 html + 标 loading（避免 effect 内同步 setState，
  // 仓库 eslint 拦）。effect 只发异步请求，setState 全在 .then/.finally 异步回调里。
  const [prevId, setPrevId] = useState(systemId)
  if (systemId !== prevId) {
    setPrevId(systemId)
    setHtml(null)
    setLoading(systemId != null)
  }

  useEffect(() => {
    if (!systemId) return
    let cancelled = false
    void getTransport()
      .call<string>("get_design_system_kit_cmd", { id: systemId })
      .then((h) => {
        if (!cancelled) setHtml(h)
      })
      .catch((e) => {
        if (!cancelled) logger.error("design", "DesignKitModal", "load kit failed", e)
      })
      .finally(() => {
        if (!cancelled) setLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [systemId])

  return (
    <Dialog open={systemId != null} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="flex h-[82vh] max-w-4xl flex-col gap-0 overflow-hidden p-0">
        <DialogHeader className="shrink-0 border-b px-4 py-3">
          <DialogTitle className="flex items-center gap-2 text-sm">
            <Palette className="h-4 w-4 text-primary" />
            {systemName?.trim() || t("design.kit.title", "设计系统套件")}
          </DialogTitle>
        </DialogHeader>
        <div className="relative flex-1 bg-muted/30">
          {loading && (
            <div
              role="status"
              aria-live="polite"
              className="absolute inset-0 flex items-center justify-center"
            >
              <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
              <span className="sr-only">{t("common.loading", "加载中...")}</span>
            </div>
          )}
          {html != null && (
            <iframe
              title={t("design.kit.title", "设计系统套件")}
              srcDoc={html}
              sandbox="allow-scripts"
              className="h-full w-full border-0 bg-white"
            />
          )}
        </div>
      </DialogContent>
    </Dialog>
  )
}

export default DesignKitModal
