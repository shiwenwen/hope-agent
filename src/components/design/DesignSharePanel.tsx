import { useCallback, useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Check, Copy, ExternalLink, Loader2, Link2, Share2 } from "lucide-react"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"

interface Props {
  artifactId: string
  /** Absolute base for the public link. Server mode = the browser origin. */
  origin: string
}

/**
 * 分享面板（Wave 1-②，仅 server 模式）：把「已存在的只读公开链接」显式呈现——
 * 显示 URL、可再复制、打开预览、随时停止分享。后端 create/get/revoke 均已就绪
 * （`*_design_share_cmd`），这里只补此前完全缺失的可见/可管 UI，修复「发出去收不回」。
 */
export function DesignSharePanel({ artifactId, origin }: Props) {
  const { t } = useTranslation()
  const [token, setToken] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState(false)
  const [copied, setCopied] = useState(false)
  const copiedTimer = useRef<number | null>(null)
  const tx = getTransport()

  const url = token ? `${origin.replace(/\/$/, "")}/api/design/share/${token}` : ""

  useEffect(() => {
    let alive = true
    setLoading(true)
    tx.call<{ token: string | null }>("get_design_share_cmd", { artifactId })
      .then((r) => {
        if (alive) setToken(r?.token ?? null)
      })
      .catch((e) => {
        logger.error("design", "DesignSharePanel::load", "load share failed", e)
        if (alive) toast.error(t("design.share.loadErr", "加载分享状态失败"))
      })
      .finally(() => {
        if (alive) setLoading(false)
      })
    return () => {
      alive = false
      if (copiedTimer.current) window.clearTimeout(copiedTimer.current)
    }
  }, [artifactId, tx, t])

  const create = useCallback(async () => {
    setBusy(true)
    try {
      const r = await tx.call<{ token: string }>("create_design_share_cmd", { artifactId })
      setToken(r.token)
    } catch (e) {
      logger.error("design", "DesignSharePanel::create", "create share failed", e)
      toast.error(t("design.share.failed", "分享失败"))
    } finally {
      setBusy(false)
    }
  }, [artifactId, tx, t])

  const copy = useCallback(async () => {
    if (!url) return
    try {
      await navigator.clipboard.writeText(url)
      setCopied(true)
      if (copiedTimer.current) window.clearTimeout(copiedTimer.current)
      copiedTimer.current = window.setTimeout(() => setCopied(false), 1600)
    } catch {
      toast.success(url) // 剪贴板不可用 → 直接展示链接
    }
  }, [url])

  const openPreview = useCallback(() => {
    if (url) window.open(url, "_blank", "noopener,noreferrer")
  }, [url])

  const stop = useCallback(async () => {
    setBusy(true)
    try {
      await tx.call("revoke_design_share_cmd", { artifactId })
      setToken(null)
      toast.success(t("design.share.stopped", "已停止分享"))
    } catch (e) {
      logger.error("design", "DesignSharePanel::revoke", "revoke share failed", e)
      toast.error(t("design.share.failed", "分享失败"))
    } finally {
      setBusy(false)
    }
  }, [artifactId, tx, t])

  return (
    <div className="absolute right-0 top-full z-30 mt-1 w-80 rounded-xl border border-border/60 bg-popover/95 p-3 shadow-[0_8px_30px_rgb(0,0,0,0.12)] backdrop-blur-xl">
      <div className="mb-2 flex items-center gap-1.5 text-xs font-medium text-foreground">
        <Share2 className="h-3.5 w-3.5 text-muted-foreground" />
        {t("design.share.linkTitle", "公开分享链接")}
      </div>

      {loading ? (
        <div className="flex items-center justify-center py-4">
          <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
        </div>
      ) : token ? (
        <>
          <div className="flex items-center gap-1 rounded-lg border border-border/60 bg-muted/40 px-2 py-1.5">
            <Link2 className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
            <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-muted-foreground" title={url}>
              {url}
            </span>
          </div>
          <p className="mt-1.5 text-[11px] leading-relaxed text-muted-foreground">
            {t("design.share.readonlyNote", "只读快照，任何拿到链接的人都能查看这个页面。")}
          </p>
          <div className="mt-2 flex items-center gap-1.5">
            <Button size="sm" variant="outline" className="h-7 flex-1 gap-1 text-xs" onClick={() => void copy()}>
              {copied ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
              {copied ? t("design.share.copied2", "已复制") : t("design.share.copyLink", "复制链接")}
            </Button>
            <Button size="sm" variant="outline" className="h-7 gap-1 text-xs" onClick={openPreview}>
              <ExternalLink className="h-3.5 w-3.5" />
              {t("design.share.openPreview", "打开")}
            </Button>
          </div>
          <Button
            size="sm"
            variant="ghost"
            disabled={busy}
            className="mt-1.5 h-7 w-full gap-1 text-xs text-destructive hover:bg-destructive/10 hover:text-destructive"
            onClick={() => void stop()}
          >
            {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : null}
            {t("design.share.stopSharing", "停止分享")}
          </Button>
        </>
      ) : (
        <>
          <p className="text-[11px] leading-relaxed text-muted-foreground">
            {t("design.share.emptyHint", "创建一个只读公开链接，任何拿到链接的人都能查看当前页面（可随时停止）。")}
          </p>
          <Button
            size="sm"
            disabled={busy}
            className="mt-2 h-7 w-full gap-1 text-xs"
            onClick={() => void create()}
          >
            {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Link2 className="h-3.5 w-3.5" />}
            {t("design.share.createLink", "创建公开链接")}
          </Button>
        </>
      )}
    </div>
  )
}

export default DesignSharePanel
