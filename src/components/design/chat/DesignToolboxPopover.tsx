import { useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { Blocks, Loader2, Search } from "lucide-react"

import { SearchInput } from "@/components/ui/search-input"
import { Button } from "@/components/ui/button"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"
import type { DesignRecipe } from "@/types/design"

/** 后端骨架 demo 统一画布（`recipe_demo.rs::DEMO_CANVAS_*`，两端常量须一致）。 */
const DEMO_W = 900
const DEMO_H = 620
/** 右栏预览内容宽（w-[300px] − p-3×2），iframe 按此等比缩放。 */
const PREVIEW_W = 276
const PREVIEW_SCALE = PREVIEW_W / DEMO_W

interface Props {
  recipes: DesignRecipe[]
  /** 点选一个模板 → 用它生成一条起步 prompt 填入 composer（不自动发）。 */
  onPick: (prompt: string) => void
  /** 形态本地化标签（复用 DesignView 的 kindLabel；缺省用通用）。 */
  kindLabel?: (kind: string) => string
  /** 当前生效设计系统 id——demo 骨架注入其配色；null = 骨架默认。 */
  systemId?: string | null
}

/**
 * 设计工具箱（B2-2 + demo 预览）：可搜索、按形态分组的模板面板，双栏——左列表，
 * 右栏 hover 即时渲染该模板的**骨架 demo**（后端纯形状 wireframe，注入当前设计系统
 * tokens：换系统即换配色气质；motion 类骨架带真动画）。点行或右栏按钮 → 组合起步
 * prompt 填入 composer（不自动发，human-in-loop）。
 *
 * 纯内容组件：由挂载方放进 `PopoverContent`（Radix Portal——620px 宽面板不被聊天
 * 面板的 overflow 祖先裁剪，Escape / outside-click 亦由 Radix 处理）。
 */
export function DesignToolboxPopover({ recipes, onPick, kindLabel, systemId }: Props) {
  const { t } = useTranslation()
  const [query, setQuery] = useState("")
  const label = kindLabel ?? ((k: string) => k)

  // ── 右栏预览状态：hover 目标 + demo HTML（防抖 + 缓存 + 竞态守卫）──
  // demo 缓存改用 state 派生（异步 fetch 只在回调 setState，规避 effect 体内同步 setState）；
  // 缓存 `""` = fetch 失败 sentinel（停 loading、不重试）。
  const [preview, setPreview] = useState<DesignRecipe | null>(null)
  const [demoCache, setDemoCache] = useState<Map<string, string>>(() => new Map())
  const demoKey = preview ? `${preview.id}|${systemId ?? ""}` : null
  const demoHtml = demoKey ? (demoCache.get(demoKey) ?? null) : null
  const demoLoading = demoKey != null && !demoCache.has(demoKey)

  useEffect(() => {
    if (!preview || !demoKey || demoCache.has(demoKey)) return
    const key = demoKey
    const recipeId = preview.id
    let cancelled = false
    const timer = window.setTimeout(() => {
      void getTransport()
        .call<string>("get_design_recipe_demo_cmd", {
          id: recipeId,
          systemId: systemId ?? null,
        })
        .then((html) => {
          if (!cancelled) setDemoCache((prev) => (prev.has(key) ? prev : new Map(prev).set(key, html)))
        })
        .catch((e) => {
          logger.error("design", "DesignToolboxPopover", "load recipe demo failed", e)
          if (!cancelled) setDemoCache((prev) => (prev.has(key) ? prev : new Map(prev).set(key, "")))
        })
    }, 150)
    return () => {
      cancelled = true
      window.clearTimeout(timer)
    }
  }, [preview, demoKey, demoCache, systemId])

  const groups = useMemo(() => {
    const q = query.trim().toLowerCase()
    const filtered = q
      ? recipes.filter(
          (r) =>
            r.name.toLowerCase().includes(q) ||
            (r.summary ?? "").toLowerCase().includes(q) ||
            (r.scenario ?? "").toLowerCase().includes(q),
        )
      : recipes
    const byKind = new Map<string, DesignRecipe[]>()
    for (const r of filtered) {
      const arr = byKind.get(r.kind)
      if (arr) arr.push(r)
      else byKind.set(r.kind, [r])
    }
    return [...byKind.entries()]
  }, [recipes, query])

  const promptFor = (r: DesignRecipe) =>
    t("design.toolbox.startPrompt", "用「{{name}}」模板做一个：{{scenario}}", {
      name: r.name,
      scenario: r.scenario || r.summary || r.name,
    })

  return (
    <div className="min-w-0">
      <div className="relative mb-2">
        <Search className="pointer-events-none absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
        <SearchInput
          autoFocus
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t("design.toolbox.search", "搜索模板 / 场景…")}
          className="h-8 pl-7 text-xs"
        />
      </div>
      <div className="flex min-h-0">
        <div className="max-h-[380px] min-w-0 flex-1 overflow-y-auto pr-1">
          {groups.length === 0 ? (
            <p className="py-4 text-center text-xs text-muted-foreground">
              {t("design.toolbox.empty", "没有匹配的模板")}
            </p>
          ) : (
            groups.map(([kind, items]) => (
              <div key={kind}>
                <div className="px-1 py-1 text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                  {label(kind)}
                </div>
                {items.map((r) => (
                  <button
                    key={r.id}
                    type="button"
                    onClick={() => onPick(promptFor(r))}
                    onMouseEnter={() => setPreview(r)}
                    onFocus={() => setPreview(r)}
                    className={cn(
                      "flex w-full flex-col gap-0.5 rounded-lg px-2 py-1.5 text-left transition-colors",
                      preview?.id === r.id
                        ? "bg-secondary/70 text-foreground"
                        : "hover:bg-secondary/40",
                    )}
                  >
                    <span className="truncate text-xs font-medium">{r.name}</span>
                    {r.summary && (
                      <span className="truncate text-[11px] text-muted-foreground">
                        {r.summary}
                      </span>
                    )}
                  </button>
                ))}
              </div>
            ))
          )}
        </div>
        <div className="ml-2 w-[300px] shrink-0 rounded-lg border border-border/60 bg-background/60 p-3">
          {preview ? (
            <div className="flex h-full flex-col gap-2">
              <div
                className="relative shrink-0 overflow-hidden rounded-md border bg-white"
                style={{ height: Math.round(DEMO_H * PREVIEW_SCALE) }}
              >
                {demoHtml != null && (
                  <iframe
                    srcDoc={demoHtml}
                    sandbox=""
                    tabIndex={-1}
                    title={preview.name}
                    className="pointer-events-none absolute left-0 top-0 origin-top-left border-0"
                    style={{
                      width: DEMO_W,
                      height: DEMO_H,
                      transform: `scale(${PREVIEW_SCALE})`,
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
              <div className="min-h-0 flex-1">
                <div className="truncate text-xs font-semibold">{preview.name}</div>
                {preview.summary && (
                  <p className="mt-0.5 line-clamp-3 text-[11px] leading-relaxed text-muted-foreground">
                    {preview.summary}
                  </p>
                )}
              </div>
              <Button
                type="button"
                size="sm"
                className="h-7 w-full text-xs"
                onClick={() => onPick(promptFor(preview))}
              >
                {t("design.toolbox.useTemplate", "使用此模板")}
              </Button>
            </div>
          ) : (
            <div className="flex h-full min-h-[220px] flex-col items-center justify-center gap-2 text-center">
              <Blocks className="h-5 w-5 text-muted-foreground/60" />
              <p className="px-3 text-[11px] leading-relaxed text-muted-foreground">
                {t("design.toolbox.previewHint", "悬停模板查看结构示意——配色跟随当前设计系统")}
              </p>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}

export default DesignToolboxPopover
