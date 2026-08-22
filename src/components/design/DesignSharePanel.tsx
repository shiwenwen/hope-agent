import { useCallback, useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  Check,
  Copy,
  ExternalLink,
  Loader2,
  Link2,
  MessageSquare,
  Share2,
  Trash2,
} from "lucide-react"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import { FloatingMenu } from "@/components/ui/floating-menu"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { DesignReviewGrant } from "@/types/design"

interface Props {
  /** 常挂载、由 open 驱动显隐（统一浮层，保留退场动画）。 */
  open: boolean
  artifactId: string
  currentVersion: number
  /** Absolute base for the public link. Server mode = the browser origin. */
  origin: string
}

/**
 * 分享面板（Wave 1-②，仅 server 模式）：把「已存在的只读公开链接」显式呈现——
 * 显示 URL、可再复制、打开预览、随时停止分享。后端 create/get/revoke 均已就绪
 * （`*_design_share_cmd`），这里只补此前完全缺失的可见/可管 UI，修复「发出去收不回」。
 */
export function DesignSharePanel({ open, artifactId, currentVersion, origin }: Props) {
  const { t } = useTranslation()
  const [token, setToken] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState(false)
  const [copied, setCopied] = useState(false)
  const [reviewGrants, setReviewGrants] = useState<DesignReviewGrant[]>([])
  const [reviewRole, setReviewRole] = useState<"viewer" | "commenter">("commenter")
  const [createdReviewToken, setCreatedReviewToken] = useState<string | null>(null)
  const copiedTimer = useRef<number | null>(null)
  const tx = getTransport()

  const url = token ? `${origin.replace(/\/$/, "")}/api/design/share/${token}` : ""

  useEffect(() => {
    if (!open) {
      setToken(null)
      setReviewGrants([])
      setCreatedReviewToken(null)
      setCopied(false)
      setLoading(false)
      if (copiedTimer.current) window.clearTimeout(copiedTimer.current)
      return
    }
    let alive = true
    setLoading(true)
    setToken(null)
    setReviewGrants([])
    setCreatedReviewToken(null)
    setCopied(false)
    Promise.all([
      tx.call<{ token: string | null }>("get_design_share_cmd", { artifactId }),
      tx.call<DesignReviewGrant[]>("list_design_review_spaces_cmd", { artifactId }),
    ])
      .then(([share, rows]) => {
        if (!alive) return
        setToken(share?.token ?? null)
        setReviewGrants(Array.isArray(rows) ? rows : [])
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
  }, [open, artifactId, tx, t])

  const createReview = useCallback(async () => {
    setBusy(true)
    try {
      const created = await tx.call<{ grant: DesignReviewGrant; token: string }>(
        "create_design_review_space_cmd",
        {
          input: {
            artifactId,
            versionNumber: currentVersion,
            role: reviewRole,
            expiresAt: new Date(Date.now() + 7 * 24 * 60 * 60 * 1000).toISOString(),
          },
        },
      )
      setCreatedReviewToken(created.token)
      setReviewGrants((rows) => [created.grant, ...rows])
    } catch (e) {
      logger.error("design", "DesignSharePanel::createReview", "create review failed", e)
      toast.error(t("design.share.reviewCreateFailed", "创建评审空间失败"))
    } finally {
      setBusy(false)
    }
  }, [artifactId, currentVersion, reviewRole, tx, t])

  const revokeReview = useCallback(
    async (grantId: string) => {
      setBusy(true)
      try {
        await tx.call("revoke_design_review_space_cmd", { artifactId, grantId })
        setReviewGrants((rows) => rows.filter((row) => row.id !== grantId))
      } catch (e) {
        logger.error("design", "DesignSharePanel::revokeReview", "revoke review failed", e)
        toast.error(t("design.share.reviewRevokeFailed", "撤销评审空间失败"))
      } finally {
        setBusy(false)
      }
    },
    [artifactId, tx, t],
  )

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
    <FloatingMenu
      open={open}
      positionClassName="right-0 top-full mt-1"
      originClassName="origin-top-right"
      className="max-h-[70vh] w-96 overflow-y-auto p-3"
    >
      <div className="mb-2 flex items-center gap-1.5 text-xs font-medium text-foreground">
        <Share2 className="h-3.5 w-3.5 text-muted-foreground" />
        {t("design.share.linkTitle", "公开分享链接")}
      </div>

      {loading ? (
        <div role="status" aria-live="polite" className="flex items-center justify-center py-4">
          <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
          <span className="sr-only">{t("common.loading", "加载中...")}</span>
        </div>
      ) : token ? (
        <>
          <div className="flex items-center gap-1 rounded-lg border border-border/60 bg-muted/40 px-2 py-1.5">
            <Link2 className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
            <span
              className="min-w-0 flex-1 truncate font-mono text-[11px] text-muted-foreground"
              data-ha-title-tip={url}
            >
              {url}
            </span>
          </div>
          <p className="mt-1.5 text-[11px] leading-relaxed text-muted-foreground">
            {t("design.share.readonlyNote", "只读快照，任何拿到链接的人都能查看这个页面。")}
          </p>
          <div className="mt-2 flex items-center gap-1.5">
            <Button
              size="sm"
              variant="outline"
              className="h-7 flex-1 gap-1 text-xs"
              onClick={() => void copy()}
            >
              {copied ? (
                <Check className="h-3.5 w-3.5 text-emerald-500" />
              ) : (
                <Copy className="h-3.5 w-3.5" />
              )}
              {copied
                ? t("design.share.copied2", "已复制")
                : t("design.share.copyLink", "复制链接")}
            </Button>
            <Button size="sm" variant="outline" className="h-7 gap-1 text-xs" onClick={openPreview}>
              <ExternalLink className="h-3.5 w-3.5" />
              {t("design.share.openPreview", "打开")}
            </Button>
          </div>
          {/* 社媒分发：复用分享 URL 打开各平台 share intent（proper noun 无需 i18n）。 */}
          <div className="mt-1.5 flex items-center gap-1.5">
            <span className="shrink-0 text-[10px] text-muted-foreground">
              {t("design.share.shareTo", "分享到")}
            </span>
            {(
              [
                ["X", `https://twitter.com/intent/tweet?url=${encodeURIComponent(url)}`],
                [
                  "微博",
                  `https://service.weibo.com/share/share.php?url=${encodeURIComponent(url)}`,
                ],
                [
                  "LinkedIn",
                  `https://www.linkedin.com/sharing/share-offsite/?url=${encodeURIComponent(url)}`,
                ],
              ] as const
            ).map(([label, intent]) => (
              <Button
                key={label}
                size="sm"
                variant="outline"
                className="h-7 flex-1 gap-1 px-1 text-[11px]"
                onClick={() => window.open(intent, "_blank", "noopener,noreferrer")}
              >
                {label}
              </Button>
            ))}
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
            {t(
              "design.share.emptyHint",
              "创建一个只读公开链接，任何拿到链接的人都能查看当前页面（可随时停止）。",
            )}
          </p>
          <Button
            size="sm"
            disabled={busy}
            className="mt-2 h-7 w-full gap-1 text-xs"
            onClick={() => void create()}
          >
            {busy ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Link2 className="h-3.5 w-3.5" />
            )}
            {t("design.share.createLink", "创建公开链接")}
          </Button>
        </>
      )}

      <div className="my-3 border-t border-border/60" />
      <div className="mb-1.5 flex items-center gap-1.5 text-xs font-medium text-foreground">
        <MessageSquare className="h-3.5 w-3.5 text-muted-foreground" />
        {t("design.share.fixedReview", "固定版本评审")}
      </div>
      <p className="text-[11px] leading-relaxed text-muted-foreground">
        {t(
          "design.share.fixedReviewHint",
          "创建 7 天有效的只读或可评论凭证，始终锚定当前版本；凭证只在创建时显示一次。",
        )}
      </p>
      <div className="mt-2 flex gap-1.5">
        <Select
          value={reviewRole}
          onValueChange={(value) => setReviewRole(value as "viewer" | "commenter")}
        >
          <SelectTrigger className="h-7 flex-1 text-xs">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="commenter">{t("design.share.commenter", "可评论")}</SelectItem>
            <SelectItem value="viewer">{t("design.share.viewer", "仅查看")}</SelectItem>
          </SelectContent>
        </Select>
        <Button
          size="sm"
          className="h-7 text-xs"
          disabled={busy}
          onClick={() => void createReview()}
        >
          {t("design.share.createReview", "创建 v{{version}}", { version: currentVersion })}
        </Button>
      </div>
      {createdReviewToken && (
        <div className="mt-2 rounded-lg border border-amber-500/30 bg-amber-500/5 p-2">
          <p className="text-[10px] text-muted-foreground">
            {t("design.share.tokenOnce", "评审 Bearer 凭证（只显示一次）")}
          </p>
          <div className="mt-1 flex items-center gap-1">
            <code className="min-w-0 flex-1 truncate text-[10px]">{createdReviewToken}</code>
            <Button
              size="icon"
              variant="ghost"
              className="h-6 w-6"
              onClick={() => void navigator.clipboard.writeText(createdReviewToken)}
            >
              <Copy className="h-3.5 w-3.5" />
            </Button>
          </div>
        </div>
      )}
      {reviewGrants.length > 0 && (
        <div className="mt-2 space-y-1">
          {reviewGrants
            .filter((grant) => !grant.revokedAt)
            .map((grant) => (
              <div
                key={grant.id}
                className="flex items-center gap-2 rounded-md border border-border/60 px-2 py-1.5 text-[11px]"
              >
                <span className="min-w-0 flex-1 truncate">
                  v{grant.versionNumber} ·{" "}
                  {grant.role === "commenter"
                    ? t("design.share.commenter", "可评论")
                    : t("design.share.viewer", "仅查看")}
                </span>
                <span className="text-muted-foreground">
                  {new Date(grant.expiresAt).toLocaleDateString()}
                </span>
                <Button
                  size="icon"
                  variant="ghost"
                  className="h-6 w-6 text-destructive"
                  disabled={busy}
                  onClick={() => void revokeReview(grant.id)}
                >
                  <Trash2 className="h-3.5 w-3.5" />
                </Button>
              </div>
            ))}
        </div>
      )}
    </FloatingMenu>
  )
}

export default DesignSharePanel
