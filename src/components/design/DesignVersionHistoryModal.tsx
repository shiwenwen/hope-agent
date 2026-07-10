import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Check, Copy, Download, Hand, History, Loader2, Monitor, RotateCcw, Search, Smartphone, Sparkles, Tablet, TriangleAlert, Undo2 } from "lucide-react"

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
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"
import type { DesignArtifactVersion, VersionOrigin } from "@/types/design"
import { toast } from "sonner"

interface Props {
  open: boolean
  onClose: () => void
  artifactId: string | null
  /** 当前工作版本号（该行不可恢复，标 Current）。 */
  currentVersion: number
  /** 恢复成功后回调（父级刷新预览 / 版本列表 / bodyHash）。 */
  onRestored: () => void
}

/** 溯源标签的图标 + 颜色 + 文案 key（AI / 手动 / 回滚）。旧行 origin 缺省时不显示徽标。 */
function originMeta(origin: VersionOrigin | undefined) {
  switch (origin) {
    case "ai":
      return {
        Icon: Sparkles,
        cls: "bg-primary/10 text-primary ring-primary/20",
        key: "design.ver.originAi",
        fallback: "AI 生成",
      }
    case "manual":
      return {
        Icon: Hand,
        cls: "bg-amber-500/10 text-amber-600 ring-amber-500/20 dark:text-amber-400",
        key: "design.ver.originManual",
        fallback: "手动编辑",
      }
    case "restore":
      return {
        Icon: Undo2,
        cls: "bg-violet-500/10 text-violet-600 ring-violet-500/20 dark:text-violet-400",
        key: "design.ver.originRestore",
        fallback: "回滚",
      }
    default:
      return null
  }
}

/**
 * 版本历史双栏模态（B3-3，源码级对标参照 `FileVersionManagerModal`）：左栏版本列表
 * （溯源徽标 / 标题 / 相对时间 / 搜索），右栏选中版本的 srcdoc **live 预览**（沙箱 iframe）。
 *
 * 性能：内容按版本号缓存（`cacheRef`），列表项 hover 预取（`prime`），选中零往返闪烁。
 * 恢复走二次确认（AlertDialog）；恢复在后端生成**新**版本（原历史不动）。
 */
export function DesignVersionHistoryModal({
  open,
  onClose,
  artifactId,
  currentVersion,
  onRestored,
}: Props) {
  const { t } = useTranslation()
  const [versions, setVersions] = useState<DesignArtifactVersion[]>([])
  const [loadingList, setLoadingList] = useState(false)
  const [selected, setSelected] = useState<number | null>(null)
  const [html, setHtml] = useState<string | null>(null)
  const [loadingContent, setLoadingContent] = useState(false)
  const [query, setQuery] = useState("")
  const [restoring, setRestoring] = useState(false)
  // 版本预览视口切换（决策增量）：在恢复前确认历史版本在手机/平板下的样子。复用主预览的设备宽度。
  const [verViewport, setVerViewport] = useState<"desktop" | "tablet" | "mobile">("desktop")
  const [confirm, setConfirm] = useState(false)
  const [promptOpen, setPromptOpen] = useState(false)
  const [copied, setCopied] = useState(false)
  // 版本号 → 快照 HTML 缓存（跨选择复用，hover 预取填充）。
  const cacheRef = useRef<Map<number, string>>(new Map())

  // 渲染期重置：打开 / 换产物立即清空（避免 effect 内同步 setState，仓库 eslint 拦）。
  const [prevKey, setPrevKey] = useState<string | null>(null)
  const [listError, setListError] = useState(false) // Wave 2-⑨：加载失败显式态（区别于「暂无版本」）
  const [reloadTick, setReloadTick] = useState(0) // 重试触发
  const openKey = open && artifactId ? artifactId : null
  if (openKey !== prevKey) {
    setPrevKey(openKey)
    setVersions([])
    setSelected(null)
    setHtml(null)
    setQuery("")
    setPromptOpen(false)
    setListError(false)
    cacheRef.current = new Map()
    setLoadingList(openKey != null)
  }

  // 拉版本列表 + 默认选中最新版本。reloadTick 变化 = 重试。
  useEffect(() => {
    if (!open || !artifactId) return
    let cancelled = false
    setLoadingList(true)
    setListError(false)
    void getTransport()
      .call<DesignArtifactVersion[]>("list_design_artifact_versions_cmd", { id: artifactId })
      .then((list) => {
        if (cancelled) return
        const rows = list ?? []
        setVersions(rows)
        // 默认选中当前工作版本（若在列表），否则最新一条。
        const pick = rows.find((v) => v.versionNumber === currentVersion) ?? rows[0]
        setSelected(pick ? pick.versionNumber : null)
      })
      .catch((e) => {
        // Wave 2-⑨：失败置显式 error 态（+ 重试），不再静默记日志伪装成「暂无版本」。
        if (!cancelled) {
          logger.error("design", "VersionHistory", "list versions failed", e)
          setListError(true)
        }
      })
      .finally(() => {
        if (!cancelled) setLoadingList(false)
      })
    return () => {
      cancelled = true
    }
  }, [open, artifactId, currentVersion, reloadTick])

  // 取某版本快照 HTML（带缓存）。
  const fetchHtml = useCallback(
    async (versionNumber: number): Promise<string | null> => {
      const cached = cacheRef.current.get(versionNumber)
      if (cached != null) return cached
      if (!artifactId) return null
      try {
        const h = await getTransport().call<string>("get_design_artifact_version_html_cmd", {
          artifactId,
          versionNumber,
        })
        cacheRef.current.set(versionNumber, h ?? "")
        return h ?? ""
      } catch (e) {
        logger.error("design", "VersionHistory", "load version html failed", e)
        return null
      }
    },
    [artifactId],
  )

  // hover 预取（不改选中态，只暖缓存）。
  const prime = useCallback(
    (versionNumber: number) => {
      if (cacheRef.current.has(versionNumber)) return
      void fetchHtml(versionNumber)
    },
    [fetchHtml],
  )

  // 选中变化 → 载右栏预览（缓存命中即无往返）。
  useEffect(() => {
    if (selected == null) {
      setHtml(null)
      setLoadingContent(false)
      return
    }
    let cancelled = false
    const cached = cacheRef.current.get(selected)
    if (cached != null) {
      setHtml(cached)
      // 命中缓存必须清 loading——否则「在途未缓存请求 → 切回已缓存版本」时，被取消的
      // 在途 finally 被 cancelled 守卫跳过，spinner 会永久盖在已正确渲染的预览上（review #1）。
      setLoadingContent(false)
      return
    }
    setLoadingContent(true)
    void fetchHtml(selected)
      .then((h) => {
        if (!cancelled) setHtml(h)
      })
      .finally(() => {
        if (!cancelled) setLoadingContent(false)
      })
    return () => {
      cancelled = true
    }
  }, [selected, fetchHtml])

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase()
    if (!q) return versions
    return versions.filter((v) => {
      const title = v.promptSummary || v.message || `v${v.versionNumber}`
      const om = originMeta(v.origin)
      const originLabel = om ? t(om.key, om.fallback) : ""
      return (
        title.toLowerCase().includes(q) ||
        `v${v.versionNumber}`.includes(q) ||
        originLabel.toLowerCase().includes(q)
      )
    })
  }, [versions, query, t])

  const selectedVersion = versions.find((v) => v.versionNumber === selected) ?? null
  const canRestore = selectedVersion != null && selectedVersion.versionNumber !== currentVersion

  const doRestore = useCallback(async () => {
    if (!artifactId || selected == null) return
    setRestoring(true)
    try {
      await getTransport().call("restore_design_version_cmd", {
        artifactId,
        versionId: selected,
      })
      toast.success(t("design.ok.restored", "已恢复到该版本"))
      setConfirm(false)
      onRestored()
      onClose()
    } catch (e) {
      logger.error("design", "VersionHistory", "restore failed", e)
      toast.error(t("design.err.restore", "恢复失败"))
    } finally {
      setRestoring(false)
    }
  }, [artifactId, selected, t, onRestored, onClose])

  const copyPrompt = useCallback(async () => {
    if (!selectedVersion?.promptSummary) return
    try {
      await navigator.clipboard.writeText(selectedVersion.promptSummary)
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1500)
    } catch (e) {
      logger.error("design", "VersionHistory", "copy prompt failed", e)
    }
  }, [selectedVersion])

  const showSearch = versions.length > 3
  const selMeta = originMeta(selectedVersion?.origin)

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="flex h-[82vh] max-w-5xl flex-col gap-0 overflow-hidden p-0">
        <DialogHeader className="shrink-0 border-b px-4 py-3">
          <DialogTitle className="flex items-center gap-2 text-sm">
            <History className="h-4 w-4 text-primary" />
            {t("design.history", "版本历史")}
          </DialogTitle>
        </DialogHeader>

        <div className="flex min-h-0 flex-1">
          {/* 左栏：版本列表 */}
          <div className="flex w-[280px] shrink-0 flex-col border-r">
            {showSearch && (
              <div className="relative border-b p-2">
                <Search className="pointer-events-none absolute left-4 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
                <Input
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  placeholder={t("design.ver.search", "搜索版本…")}
                  className="h-8 pl-7 text-xs"
                />
              </div>
            )}
            <div className="min-h-0 flex-1 overflow-y-auto p-1.5">
              {loadingList ? (
                <div className="flex justify-center py-10">
                  <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
                </div>
              ) : listError ? (
                // Wave 2-⑨：加载失败显式态（区别于「暂无版本」），带重试。
                <div className="flex flex-col items-center gap-2 py-10 text-center">
                  <TriangleAlert className="h-5 w-5 text-amber-500" />
                  <p className="text-xs text-muted-foreground">
                    {t("design.ver.loadFailed", "版本列表加载失败")}
                  </p>
                  <Button
                    size="sm"
                    variant="outline"
                    className="h-7 gap-1 text-xs"
                    onClick={() => setReloadTick((k) => k + 1)}
                  >
                    <RotateCcw className="h-3 w-3" />
                    {t("common.retry", "重试")}
                  </Button>
                </div>
              ) : filtered.length === 0 ? (
                <div className="py-10 text-center text-xs text-muted-foreground">
                  {versions.length === 0
                    ? t("design.noVersions", "暂无版本")
                    : t("design.ver.noMatch", "无匹配版本")}
                </div>
              ) : (
                filtered.map((v) => {
                  const om = originMeta(v.origin)
                  const title = v.promptSummary || v.message || t("design.version", "版本")
                  const isCurrent = v.versionNumber === currentVersion
                  const active = v.versionNumber === selected
                  return (
                    <button
                      key={v.versionNumber}
                      type="button"
                      onClick={() => setSelected(v.versionNumber)}
                      onMouseEnter={() => prime(v.versionNumber)}
                      onFocus={() => prime(v.versionNumber)}
                      className={cn(
                        "mb-1 flex w-full flex-col gap-1 rounded-lg border px-2.5 py-2 text-left transition-colors",
                        active
                          ? "border-primary/50 bg-primary/5"
                          : "border-transparent hover:bg-secondary/60",
                      )}
                    >
                      <div className="flex items-center gap-1.5">
                        <span className="font-mono text-[11px] text-muted-foreground">
                          v{v.versionNumber}
                        </span>
                        {isCurrent && (
                          <span className="rounded-full bg-emerald-500/10 px-1.5 py-px text-[10px] font-medium text-emerald-600 ring-1 ring-inset ring-emerald-500/20 dark:text-emerald-400">
                            {t("design.ver.current", "当前")}
                          </span>
                        )}
                        {om && (
                          <span
                            className={cn(
                              "inline-flex items-center gap-0.5 rounded-full px-1.5 py-px text-[10px] font-medium ring-1 ring-inset",
                              om.cls,
                            )}
                          >
                            <om.Icon className="h-2.5 w-2.5" />
                            {v.origin === "restore" && /v(\d+)/.exec(v.message ?? "")
                              ? t("design.ver.restoredFrom", "恢复自 v{{n}}", {
                                  n: /v(\d+)/.exec(v.message ?? "")![1],
                                })
                              : t(om.key, om.fallback)}
                          </span>
                        )}
                      </div>
                      <span className="truncate text-xs font-medium">{title}</span>
                      <span className="text-[11px] text-muted-foreground">
                        {new Date(v.createdAt).toLocaleString()}
                      </span>
                    </button>
                  )
                })
              )}
            </div>
          </div>

          {/* 右栏：选中版本 live 预览 */}
          <div className="flex min-w-0 flex-1 flex-col">
            {selectedVersion == null ? (
              <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
                {t("design.ver.selectHint", "选择一个版本以预览")}
              </div>
            ) : (
              <>
                <div className="flex shrink-0 items-center gap-2 border-b px-3 py-2">
                  <span className="font-mono text-xs text-muted-foreground">
                    v{selectedVersion.versionNumber}
                  </span>
                  {selMeta && (
                    <span
                      className={cn(
                        "inline-flex items-center gap-0.5 rounded-full px-1.5 py-0.5 text-[10px] font-medium ring-1 ring-inset",
                        selMeta.cls,
                      )}
                    >
                      <selMeta.Icon className="h-2.5 w-2.5" />
                      {t(selMeta.key, selMeta.fallback)}
                    </span>
                  )}
                  <span className="truncate text-xs text-muted-foreground">
                    {new Date(selectedVersion.createdAt).toLocaleString()}
                  </span>
                  <div className="ml-auto flex items-center gap-1.5">
                    {selectedVersion.promptSummary && (
                      <Button
                        variant="ghost"
                        size="sm"
                        className="h-7 gap-1 text-xs"
                        onClick={() => setPromptOpen((o) => !o)}
                      >
                        {t("design.ver.prompt", "Prompt")}
                      </Button>
                    )}
                    {html != null && (
                      <Button
                        variant="outline"
                        size="sm"
                        className="h-7 gap-1.5 text-xs"
                        onClick={() => {
                          // 逐版本下载：把选中版本的 srcDoc 存成 html 文件（取出历史版本，无需恢复）。
                          const blob = new Blob([html], { type: "text/html" })
                          const url = URL.createObjectURL(blob)
                          const a = document.createElement("a")
                          a.href = url
                          a.download = `v${selected}.html`
                          a.click()
                          URL.revokeObjectURL(url)
                        }}
                      >
                        <Download className="h-3.5 w-3.5" />
                        {t("common.download", "下载")}
                      </Button>
                    )}
                    {canRestore && (
                      <Button
                        size="sm"
                        className="h-7 gap-1.5"
                        disabled={restoring}
                        onClick={() => setConfirm(true)}
                      >
                        <RotateCcw className="h-3.5 w-3.5" />
                        {t("design.restore", "恢复")}
                      </Button>
                    )}
                  </div>
                </div>

                {promptOpen && selectedVersion.promptSummary && (
                  <div className="shrink-0 border-b bg-muted/40 px-3 py-2">
                    <div className="mb-1 flex items-center justify-between">
                      <span className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
                        {t("design.ver.prompt", "Prompt")}
                      </span>
                      <Button
                        variant="ghost"
                        size="sm"
                        className="h-6 gap-1 text-[11px]"
                        onClick={() => void copyPrompt()}
                      >
                        {copied ? (
                          <Check className="h-3 w-3 text-emerald-500" />
                        ) : (
                          <Copy className="h-3 w-3" />
                        )}
                        {copied ? t("common.copied", "已复制") : t("common.copy", "复制")}
                      </Button>
                    </div>
                    <p className="max-h-24 overflow-y-auto whitespace-pre-wrap text-xs leading-relaxed text-foreground/80">
                      {selectedVersion.promptSummary}
                    </p>
                  </div>
                )}

                <div className="relative min-h-0 flex-1 bg-muted/30">
                  {/* 视口切换：desktop 全宽 / tablet 820 / mobile 390，非 desktop 居中设备框。 */}
                  <div className="absolute right-2 top-2 z-10 flex gap-0.5 rounded-md border bg-background/90 p-0.5 shadow-sm">
                    {(
                      [
                        ["desktop", Monitor],
                        ["tablet", Tablet],
                        ["mobile", Smartphone],
                      ] as const
                    ).map(([v, Icon]) => (
                      <button
                        key={v}
                        type="button"
                        onClick={() => setVerViewport(v)}
                        aria-label={v}
                        aria-pressed={verViewport === v}
                        className={cn(
                          "flex h-6 w-6 items-center justify-center rounded",
                          verViewport === v
                            ? "bg-primary text-primary-foreground"
                            : "text-muted-foreground hover:bg-muted",
                        )}
                      >
                        <Icon className="h-3.5 w-3.5" />
                      </button>
                    ))}
                  </div>
                  {loadingContent && (
                    <div
                      role="status"
                      aria-live="polite"
                      className="absolute inset-0 flex items-center justify-center"
                    >
                      <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
                      <span className="sr-only">{t("common.loading", "加载中...")}</span>
                    </div>
                  )}
                  {html != null &&
                    (verViewport === "desktop" ? (
                      <iframe
                        title={t("design.history", "版本历史")}
                        srcDoc={html}
                        sandbox="allow-scripts"
                        className="h-full w-full border-0 bg-white"
                      />
                    ) : (
                      <div className="flex h-full items-center justify-center overflow-auto p-4">
                        <iframe
                          title={t("design.history", "版本历史")}
                          srcDoc={html}
                          sandbox="allow-scripts"
                          style={{ width: verViewport === "tablet" ? 820 : 390 }}
                          className="h-full max-h-full shrink-0 rounded-[1.25rem] border-[6px] border-neutral-800 bg-white shadow-xl dark:border-neutral-700"
                        />
                      </div>
                    ))}
                </div>
              </>
            )}
          </div>
        </div>
      </DialogContent>

      <AlertDialog open={confirm} onOpenChange={(o) => !o && setConfirm(false)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("design.ver.restoreTitle", "恢复到该版本？")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t(
                "design.ver.restoreHelp",
                "会以该版本内容生成一个新版本，当前内容仍保留在历史中，可再切回。",
              )}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={restoring}>{t("common.cancel", "取消")}</AlertDialogCancel>
            <AlertDialogAction
              onClick={(e) => {
                e.preventDefault()
                void doRestore()
              }}
              disabled={restoring}
            >
              {restoring ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <RotateCcw className="h-4 w-4" />
              )}
              {t("design.restore", "恢复")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </Dialog>
  )
}

export default DesignVersionHistoryModal
