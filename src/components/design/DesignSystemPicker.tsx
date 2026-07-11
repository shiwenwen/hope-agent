import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { Check, Eye, Loader2, Palette, Pencil, Search, Star, Trash2, X } from "lucide-react"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
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
import { Input } from "@/components/ui/input"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { toast } from "sonner"
import { cn } from "@/lib/utils"
import type { DesignSystemFull, DesignSystemMeta } from "@/types/design"

// 固定分组顺序：原创原型 → 品牌品类（与后端 `brands.rs` cat() 一致）→ 「我的」垫底。
// 后端类目为数据字符串，未知类目排在「我的」之前。
const GROUP_ORDER = [
  "原创原型",
  "开发者工具",
  "AI 产品",
  "SaaS / 生产力",
  "设计 / 框架",
  "社交 / 消费",
  "媒体 / 电商",
  "大厂 / 应用",
]

/** 右栏预览目标哨兵：「无设计系统」。 */
const NONE_PREVIEW = "__none__"

/** 气质 demo：用最通用的落地页骨架（导航+hero+特性卡+CTA）呈现该系统渲染出来的页面
 *  长什么样（`recipe_demo.rs` 900×620 画布，iframe 等比缩放）。 */
const DEMO_RECIPE = "web-landing"
const DEMO_W = 900
const DEMO_H = 620

/** 字体栈首个 family 名（strip 引号）；空栈 null。 */
function firstFamily(stack: string | undefined): string | null {
  if (!stack) return null
  const first = stack.split(",")[0]?.trim().replace(/^['"]|['"]$/g, "")
  return first || null
}

/** 浅色判定（YIQ 亮度）：swatch chip 内 hex 小字颜色自适应。仅 #rrggbb 判定。 */
function isLightColor(c: string): boolean {
  if (!/^#[0-9a-fA-F]{6}$/.test(c)) return false
  const r = parseInt(c.slice(1, 3), 16)
  const g = parseInt(c.slice(3, 5), 16)
  const b = parseInt(c.slice(5, 7), 16)
  return 0.299 * r + 0.587 * g + 0.114 * b > 150
}

interface Props {
  systems: DesignSystemMeta[]
  /** 当前选中系统 id；`null` = 无。 */
  value: string | null
  onChange: (id: string | null) => void
  open: boolean
  onOpenChange: (open: boolean) => void
  /** 是否提供「无设计系统」选项。 */
  allowNone?: boolean
  /** 预览某系统的套件视图（Kit）；提供时右栏显示「预览套件」按钮。 */
  onPreviewKit?: (systemId: string, name: string) => void
  /** 当前「新对话默认设计系统」id；提供 onSetDefault 时该行显示「默认」态。 */
  defaultSystemId?: string | null
  /** 设为/取消新对话默认设计系统；提供时每行显示「设为默认」按钮。 */
  onSetDefault?: (systemId: string | null) => void
  /** 用户系统被重命名 / 删除后回调（宿主重载 systems）；提供时非内置系统行显示改名 / 删除按钮。 */
  onSystemsChanged?: () => void
  /** 底部附加操作（反向提取 / 导入 / 导出等）。 */
  footer?: ReactNode
}

/**
 * 可搜索 + 按品类分组的设计系统选择器（Dialog 承载，规避菜单内输入焦点冲突）。
 * 双栏：左列表（行内 4 槽微缩色条），右预览随 hover 即时切换——名称 / 类目 / 摘要 +
 * 色板大 chip（meta.swatches 零等待）+ 字体 Ag 试排（hover 拉 full tokens，Map 缓存）。
 * 预览目标优先级：hover 行 > 当前选中 > 「无设计系统」说明。
 */
export function DesignSystemPicker({
  systems,
  value,
  onChange,
  open,
  onOpenChange,
  allowNone = true,
  onPreviewKit,
  defaultSystemId,
  onSetDefault,
  onSystemsChanged,
  footer,
}: Props) {
  const { t } = useTranslation()
  const [query, setQuery] = useState("")
  const mineLabel = t("design.picker.mine", "我的设计系统")
  // 用户系统管理（W3-K）：内联改名 + 删除确认（仅 source != builtin 行显示入口）。
  const [editingId, setEditingId] = useState<string | null>(null)
  const [editingName, setEditingName] = useState("")
  const [renaming, setRenaming] = useState(false)
  const [deleting, setDeleting] = useState<DesignSystemMeta | null>(null)
  const [deletingBusy, setDeletingBusy] = useState(false)

  const beginRename = (s: DesignSystemMeta) => {
    setEditingId(s.id)
    setEditingName(s.name)
  }
  const commitRename = async () => {
    const id = editingId
    const name = editingName.trim()
    if (!id || !name || renaming) return
    setRenaming(true)
    try {
      await getTransport().call("rename_design_system_cmd", { id, name })
      setEditingId(null)
      onSystemsChanged?.()
    } catch (e) {
      logger.error("design", "DesignSystemPicker", "rename system failed", e)
      toast.error(t("design.picker.renameFailed", "重命名失败"))
    } finally {
      setRenaming(false)
    }
  }
  const confirmDelete = async () => {
    const s = deleting
    if (!s || deletingBusy) return
    setDeletingBusy(true)
    try {
      await getTransport().call("delete_design_system_cmd", { id: s.id })
      setDeleting(null)
      onSystemsChanged?.()
      toast.success(t("design.picker.deleted", "已删除设计系统"))
    } catch (e) {
      logger.error("design", "DesignSystemPicker", "delete system failed", e)
      toast.error(t("design.picker.deleteFailed", "删除失败"))
    } finally {
      setDeletingBusy(false)
    }
  }

  // ── 右栏预览状态 ──────────────────────────────────────────────
  // hover 目标（系统 id 或 NONE_PREVIEW）；null = 未 hover，回落到当前选中。
  const [hoverTarget, setHoverTarget] = useState<string | null>(null)
  // full tokens 缓存改用 **state**（异步 fetch 只在 .then/.catch 回调 setState，规避 effect 体内
  // 同步 setState）；派生 previewFull/previewLoading。缓存 `null` = fetch 失败 sentinel（停 loading、不重试）。
  const [fullCache, setFullCache] = useState<Map<string, DesignSystemFull | null>>(() => new Map())
  // 关闭时下次打开回落到选中项：渲染期依 open 跃迁重置 hover（非 effect，规避 set-state-in-effect）。
  const [prevOpen, setPrevOpen] = useState(open)
  if (open !== prevOpen) {
    setPrevOpen(open)
    if (!open) setHoverTarget(null)
  }

  const previewTarget = hoverTarget ?? value ?? NONE_PREVIEW
  // 当前生效的预览目标（关闭 / 无目标 = null）：所有右栏派生都以它为键，闭态不渲染任何预览。
  const activePreviewId = open && previewTarget !== NONE_PREVIEW ? previewTarget : null
  const previewFull = activePreviewId ? (fullCache.get(activePreviewId) ?? null) : null
  const previewLoading = activePreviewId != null && !fullCache.has(activePreviewId)
  const previewMeta = useMemo(
    () =>
      previewTarget === NONE_PREVIEW
        ? null
        : (systems.find((s) => s.id === previewTarget) ?? null),
    [systems, previewTarget],
  )

  // ── 气质 demo（web-landing 骨架注入该系统 tokens）──
  // demo HTML 缓存改用 state 派生（同上，异步 fetch 只在回调 setState）；缓存 `""` = fetch 失败 sentinel。
  const [demoCache, setDemoCache] = useState<Map<string, string>>(() => new Map())
  const demoBoxRef = useRef<HTMLDivElement | null>(null)
  const [demoScale, setDemoScale] = useState(0.4)
  const demoHtml = activePreviewId ? (demoCache.get(activePreviewId) ?? null) : null
  const demoLoading = activePreviewId != null && !demoCache.has(activePreviewId)

  // demo 容器实测宽 → 等比缩放。用 ResizeObserver（setState 在观察回调、非 effect 体内）：既满足
  // set-state-in-effect 规则、又顺带跟随 Dialog 尺寸变化重测（观察即触发首帧回调，无需同步测一次）。
  useLayoutEffect(() => {
    if (!open) return
    const el = demoBoxRef.current
    if (!el) return
    const ro = new ResizeObserver(() => {
      const w = el.clientWidth
      if (w && w > 40) setDemoScale(w / DEMO_W)
    })
    ro.observe(el)
    return () => ro.disconnect()
  }, [open, previewTarget])

  // demo fetch（防抖 + 竞态守卫）：命中缓存早退、未命中才 150ms 后拉；结果 / 失败 sentinel 均在回调落缓存。
  useEffect(() => {
    if (!activePreviewId || demoCache.has(activePreviewId)) return
    const id = activePreviewId
    let cancelled = false
    const timer = window.setTimeout(() => {
      void getTransport()
        .call<string>("get_design_recipe_demo_cmd", { id: DEMO_RECIPE, systemId: id })
        .then((html) => {
          if (!cancelled) setDemoCache((prev) => (prev.has(id) ? prev : new Map(prev).set(id, html)))
        })
        .catch((e) => {
          logger.error("design", "DesignSystemPicker", "load system demo failed", e)
          if (!cancelled) setDemoCache((prev) => (prev.has(id) ? prev : new Map(prev).set(id, "")))
        })
    }, 150)
    return () => {
      cancelled = true
      window.clearTimeout(timer)
    }
  }, [activePreviewId, demoCache])

  // hover 拉系统 full（tokens 供字体试排）：120ms 微防抖防快速滑过连发 IPC，
  // Map 缓存命中零延迟，cancelled 守卫防慢响应覆盖新目标。
  useEffect(() => {
    if (!activePreviewId || fullCache.has(activePreviewId)) return
    const id = activePreviewId
    let cancelled = false
    const timer = window.setTimeout(() => {
      void getTransport()
        .call<DesignSystemFull>("get_design_system_cmd", { id })
        .then((full) => {
          if (!cancelled) setFullCache((prev) => (prev.has(id) ? prev : new Map(prev).set(id, full)))
        })
        .catch((e) => {
          logger.error("design", "DesignSystemPicker", "load system preview failed", e)
          if (!cancelled) setFullCache((prev) => (prev.has(id) ? prev : new Map(prev).set(id, null)))
        })
    }, 120)
    return () => {
      cancelled = true
      window.clearTimeout(timer)
    }
  }, [activePreviewId, fullCache])

  // 字体试排 tiles（sans / serif / mono，缺键跳过）；full 必须匹配当前目标防陈旧渲染。
  const fontTiles = useMemo(() => {
    if (!previewFull || previewFull.id !== previewTarget) return []
    const roles: Array<[string, string]> = [
      ["--ds-font-sans", "Sans"],
      ["--ds-font-serif", "Serif"],
      ["--ds-font-mono", "Mono"],
    ]
    return roles.flatMap(([key, role]) => {
      const stack = previewFull.tokens[key]
      const family = firstFamily(stack)
      return stack && family ? [{ role, family, stack }] : []
    })
  }, [previewFull, previewTarget])

  const groups = useMemo(() => {
    const q = query.trim().toLowerCase()
    const filtered = q
      ? systems.filter(
          (s) =>
            s.name.toLowerCase().includes(q) ||
            (s.summary ?? "").toLowerCase().includes(q) ||
            (s.category ?? "").toLowerCase().includes(q),
        )
      : systems
    const byGroup = new Map<string, DesignSystemMeta[]>()
    for (const s of filtered) {
      const key = s.category || mineLabel
      const arr = byGroup.get(key)
      if (arr) arr.push(s)
      else byGroup.set(key, [s])
    }
    const rank = (k: string) => {
      const i = GROUP_ORDER.indexOf(k)
      if (i >= 0) return i
      if (k === mineLabel) return GROUP_ORDER.length + 1
      return GROUP_ORDER.length
    }
    return [...byGroup.entries()].sort((a, b) => rank(a[0]) - rank(b[0]))
  }, [systems, query, mineLabel])

  const pick = (id: string | null) => {
    onChange(id)
    onOpenChange(false)
  }

  return (
    <>
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-3xl gap-0 overflow-hidden p-0">
        <DialogHeader className="border-b px-4 py-3">
          <DialogTitle className="flex items-center gap-2 text-sm">
            <Palette className="h-4 w-4 text-primary" />
            {t("design.picker.title", "选择设计系统")}
          </DialogTitle>
        </DialogHeader>
        <div className="relative border-b px-3 py-2">
          <Search className="pointer-events-none absolute left-5 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            autoFocus
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("design.picker.search", "搜索品牌 / 风格…")}
            className="h-9 pl-8"
          />
        </div>
        <div className="flex min-h-0">
          <div className="h-[52vh] min-w-0 flex-1 overflow-y-auto px-1.5 pb-1.5 md:max-w-[46%] md:border-r">
            {allowNone && (
              <button
                type="button"
                onClick={() => pick(null)}
                onMouseEnter={() => setHoverTarget(NONE_PREVIEW)}
                onFocus={() => setHoverTarget(NONE_PREVIEW)}
                className={cn(
                  "mt-1.5 flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-left text-sm hover:bg-accent",
                  value == null && "bg-accent",
                )}
              >
                <span className="flex-1">{t("design.systemNone", "无设计系统")}</span>
                {value == null && <Check className="h-4 w-4 text-primary" />}
              </button>
            )}
            {groups.length === 0 && (
              <p className="px-3 py-8 text-center text-sm text-muted-foreground">
                {t("design.picker.noMatch", "无匹配的设计系统")}
              </p>
            )}
            {groups.map(([group, items]) => (
              <div key={group} className="mb-1">
                <div className="sticky top-0 z-10 bg-background/95 px-2.5 py-1.5 text-sm font-semibold text-foreground backdrop-blur">
                  {group}{" "}
                  <span className="font-normal text-muted-foreground">· {items.length}</span>
                </div>
                {items.map((s) => (
                  <div
                    key={s.id}
                    className={cn(
                      "group/sys flex items-center gap-1 rounded-md pr-1 hover:bg-accent",
                      value === s.id && "bg-accent",
                    )}
                  >
                    {editingId === s.id ? (
                      <div className="flex min-w-0 flex-1 items-center gap-1 px-2 py-1.5">
                        <Input
                          autoFocus
                          value={editingName}
                          onChange={(e) => setEditingName(e.target.value)}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") {
                              e.preventDefault()
                              void commitRename()
                            } else if (e.key === "Escape") {
                              e.preventDefault()
                              setEditingId(null)
                            }
                          }}
                          className="h-7 flex-1 text-sm"
                        />
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon"
                          className="h-7 w-7 shrink-0"
                          disabled={renaming || !editingName.trim()}
                          onClick={() => void commitRename()}
                        >
                          {renaming ? (
                            <Loader2 className="h-4 w-4 animate-spin" />
                          ) : (
                            <Check className="h-4 w-4 text-primary" />
                          )}
                        </Button>
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon"
                          className="h-7 w-7 shrink-0"
                          onClick={() => setEditingId(null)}
                        >
                          <X className="h-4 w-4" />
                        </Button>
                      </div>
                    ) : (
                      <>
                        <button
                          type="button"
                          onClick={() => pick(s.id)}
                          onMouseEnter={() => setHoverTarget(s.id)}
                          onFocus={() => setHoverTarget(s.id)}
                          className="flex min-w-0 flex-1 items-start gap-2 px-2.5 py-1.5 text-left"
                        >
                          <div className="min-w-0 flex-1">
                            <div className="flex items-center gap-1.5">
                              <span className="truncate text-sm">{s.name}</span>
                              {onSetDefault && defaultSystemId === s.id && (
                                <span className="shrink-0 rounded bg-primary/10 px-1 text-[10px] font-medium text-primary">
                                  {t("design.default", "默认")}
                                </span>
                              )}
                              {(s.swatches?.length ?? 0) > 0 && (
                                <span className="ml-auto flex shrink-0 gap-0.5 pl-1.5">
                                  {s.swatches!.slice(0, 4).map((c, i) => (
                                    <span
                                      key={i}
                                      className="h-2.5 w-2.5 rounded-[3px] border border-black/10 dark:border-white/15"
                                      style={{ background: c }}
                                    />
                                  ))}
                                </span>
                              )}
                            </div>
                            {s.summary && (
                              <div className="truncate text-xs text-muted-foreground">
                                {s.summary}
                              </div>
                            )}
                          </div>
                          {value === s.id && (
                            <Check className="mt-0.5 h-4 w-4 shrink-0 text-primary" />
                          )}
                        </button>
                        {onSetDefault && (
                          <IconTip
                            label={
                              defaultSystemId === s.id
                                ? t("design.unsetDefault", "取消默认")
                                : t("design.setDefault", "设为新对话默认")
                            }
                            side="left"
                          >
                            <Button
                              type="button"
                              variant="ghost"
                              size="icon"
                              className={cn(
                                "h-7 w-7 shrink-0 transition-opacity",
                                defaultSystemId === s.id
                                  ? "text-primary opacity-100"
                                  : "opacity-0 group-hover/sys:opacity-100",
                              )}
                              onClick={(e) => {
                                e.stopPropagation()
                                onSetDefault(defaultSystemId === s.id ? null : s.id)
                              }}
                            >
                              <Star
                                className="h-4 w-4"
                                fill={defaultSystemId === s.id ? "currentColor" : "none"}
                              />
                            </Button>
                          </IconTip>
                        )}
                        {onPreviewKit && (
                          <IconTip label={t("design.kit.preview", "预览套件")} side="left">
                            <Button
                              type="button"
                              variant="ghost"
                              size="icon"
                              className="h-7 w-7 shrink-0 opacity-0 transition-opacity group-hover/sys:opacity-100 md:hidden"
                              onClick={(e) => {
                                e.stopPropagation()
                                onPreviewKit(s.id, s.name)
                              }}
                            >
                              <Eye className="h-4 w-4" />
                            </Button>
                          </IconTip>
                        )}
                        {/* 用户系统管理（W3-K）：仅非内置行、且宿主提供 onSystemsChanged 时显示 */}
                        {onSystemsChanged && s.source !== "builtin" && (
                          <>
                            <IconTip label={t("design.picker.rename", "重命名")} side="left">
                              <Button
                                type="button"
                                variant="ghost"
                                size="icon"
                                className="h-7 w-7 shrink-0 opacity-0 transition-opacity group-hover/sys:opacity-100"
                                onClick={(e) => {
                                  e.stopPropagation()
                                  beginRename(s)
                                }}
                              >
                                <Pencil className="h-4 w-4" />
                              </Button>
                            </IconTip>
                            <IconTip label={t("common.delete", "删除")} side="left">
                              <Button
                                type="button"
                                variant="ghost"
                                size="icon"
                                className="h-7 w-7 shrink-0 text-destructive opacity-0 transition-opacity hover:text-destructive group-hover/sys:opacity-100"
                                onClick={(e) => {
                                  e.stopPropagation()
                                  setDeleting(s)
                                }}
                              >
                                <Trash2 className="h-4 w-4" />
                              </Button>
                            </IconTip>
                          </>
                        )}
                      </>
                    )}
                  </div>
                ))}
              </div>
            ))}
          </div>
          <div className="hidden h-[52vh] min-w-0 flex-1 md:block">
            {previewTarget === NONE_PREVIEW ? (
              <div className="flex h-full flex-col p-4">
                <div className="text-sm font-semibold">
                  {t("design.systemNone", "无设计系统")}
                </div>
                <p className="mt-2 text-xs leading-relaxed text-muted-foreground">
                  {t(
                    "design.picker.noneSummary",
                    "不绑定设计系统，AI 按内容自由决定视觉风格。",
                  )}
                </p>
              </div>
            ) : previewMeta ? (
              <div className="flex h-full flex-col gap-3 overflow-y-auto p-4">
                <div>
                  <div className="flex items-center gap-2">
                    <span className="truncate text-sm font-semibold">{previewMeta.name}</span>
                    {previewMeta.category && (
                      <span className="shrink-0 rounded-full bg-muted px-2 py-0.5 text-[10px] text-muted-foreground">
                        {previewMeta.category}
                      </span>
                    )}
                  </div>
                  {previewMeta.summary && (
                    <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
                      {previewMeta.summary}
                    </p>
                  )}
                </div>
                <div
                  ref={demoBoxRef}
                  className="relative shrink-0 overflow-hidden rounded-md border bg-white"
                  style={{ height: Math.round(DEMO_H * demoScale) }}
                >
                  {demoHtml != null && (
                    <iframe
                      srcDoc={demoHtml}
                      sandbox=""
                      tabIndex={-1}
                      title={previewMeta.name}
                      className="pointer-events-none absolute left-0 top-0 origin-top-left border-0"
                      style={{
                        width: DEMO_W,
                        height: DEMO_H,
                        transform: `scale(${demoScale})`,
                      }}
                    />
                  )}
                  {demoLoading && (
                    <div
                      role="status"
                      aria-live="polite"
                      className="absolute inset-0 flex items-center justify-center"
                    >
                      <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
                      <span className="sr-only">{t("common.loading", "加载中...")}</span>
                    </div>
                  )}
                </div>
                {(previewMeta.swatches?.length ?? 0) > 0 && (
                  <div>
                    <div className="mb-1.5 text-xs font-medium text-muted-foreground">
                      {t("design.picker.palette", "色板")}
                    </div>
                    <div className="grid grid-cols-4 gap-1.5">
                      {previewMeta.swatches!.map((c, i) => (
                        <div
                          key={i}
                          className="flex h-14 items-end justify-center rounded-md border p-1"
                          style={{ background: c }}
                        >
                          {c.startsWith("#") && (
                            <span
                              className="font-mono text-[9px]"
                              style={{
                                color: isLightColor(c)
                                  ? "rgba(0,0,0,.65)"
                                  : "rgba(255,255,255,.9)",
                              }}
                            >
                              {c}
                            </span>
                          )}
                        </div>
                      ))}
                    </div>
                  </div>
                )}
                {(previewLoading || fontTiles.length > 0) && (
                  <div>
                    <div className="mb-1.5 text-xs font-medium text-muted-foreground">
                      {t("design.picker.typography", "字体")}
                    </div>
                    {previewLoading ? (
                      <div className="grid grid-cols-3 gap-1.5">
                        {[0, 1, 2].map((i) => (
                          <div key={i} className="h-[72px] animate-pulse rounded-md bg-muted" />
                        ))}
                      </div>
                    ) : (
                      <div className="grid grid-cols-3 gap-1.5">
                        {fontTiles.map((f) => (
                          <div key={f.role} className="min-w-0 rounded-md border p-2">
                            <div className="text-xl leading-tight" style={{ fontFamily: f.stack }}>
                              Ag
                            </div>
                            <div className="mt-1 truncate text-[10px] font-medium">{f.family}</div>
                            <div className="text-[9px] text-muted-foreground">{f.role}</div>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                )}
                {onPreviewKit && (
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="mt-auto shrink-0"
                    onClick={() => onPreviewKit(previewMeta.id, previewMeta.name)}
                  >
                    <Eye className="mr-1.5 h-3.5 w-3.5" />
                    {t("design.kit.preview", "预览套件")}
                  </Button>
                )}
              </div>
            ) : null}
          </div>
        </div>
        {footer && <div className="border-t p-2">{footer}</div>}
      </DialogContent>
    </Dialog>

    {/* 删除用户设计系统确认（W3-K） */}
    <AlertDialog open={deleting != null} onOpenChange={(o) => !o && !deletingBusy && setDeleting(null)}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("design.picker.deleteTitle", "删除设计系统？")}</AlertDialogTitle>
          <AlertDialogDescription>
            {t(
              "design.picker.deleteHint",
              "将删除「{{name}}」及其 Token / 资产，不可撤销；已使用它的产物保持不变。",
              { name: deleting?.name ?? "" },
            )}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel disabled={deletingBusy}>
            {t("common.cancel", "取消")}
          </AlertDialogCancel>
          <AlertDialogAction
            className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            disabled={deletingBusy}
            onClick={(e) => {
              e.preventDefault()
              void confirmDelete()
            }}
          >
            {deletingBusy && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
            {t("common.delete", "删除")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
    </>
  )
}
