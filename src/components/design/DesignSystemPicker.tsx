import { useMemo, useState, type ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { Check, Eye, Palette, Search } from "lucide-react"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import type { DesignSystemMeta } from "@/types/design"

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

interface Props {
  systems: DesignSystemMeta[]
  /** 当前选中系统 id；`null` = 无。 */
  value: string | null
  onChange: (id: string | null) => void
  open: boolean
  onOpenChange: (open: boolean) => void
  /** 是否提供「无设计系统」选项。 */
  allowNone?: boolean
  /** 预览某系统的套件视图（Kit）；提供时每行显示「预览套件」按钮。 */
  onPreviewKit?: (systemId: string, name: string) => void
  /** 底部附加操作（反向提取 / 导入 / 导出等）。 */
  footer?: ReactNode
}

/** 可搜索 + 按品类分组的设计系统选择器（Dialog 承载，规避菜单内输入焦点冲突）。 */
export function DesignSystemPicker({
  systems,
  value,
  onChange,
  open,
  onOpenChange,
  allowNone = true,
  onPreviewKit,
  footer,
}: Props) {
  const { t } = useTranslation()
  const [query, setQuery] = useState("")
  const mineLabel = t("design.picker.mine", "我的设计系统")

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
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg gap-0 overflow-hidden p-0">
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
        <div className="h-[52vh] overflow-y-auto px-1.5 pb-1.5">
          {allowNone && (
            <button
              type="button"
              onClick={() => pick(null)}
              className={cn(
                "flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-left text-sm hover:bg-accent",
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
              <div className="sticky top-0 z-10 bg-background/95 px-2.5 py-1 text-xs font-medium text-muted-foreground backdrop-blur">
                {group} · {items.length}
              </div>
              {items.map((s) => (
                <div
                  key={s.id}
                  className={cn(
                    "group/sys flex items-center gap-1 rounded-md pr-1 hover:bg-accent",
                    value === s.id && "bg-accent",
                  )}
                >
                  <button
                    type="button"
                    onClick={() => pick(s.id)}
                    className="flex min-w-0 flex-1 items-start gap-2 px-2.5 py-1.5 text-left"
                  >
                    <div className="min-w-0 flex-1">
                      <div className="truncate text-sm">{s.name}</div>
                      {s.summary && (
                        <div className="truncate text-xs text-muted-foreground">{s.summary}</div>
                      )}
                    </div>
                    {value === s.id && <Check className="mt-0.5 h-4 w-4 shrink-0 text-primary" />}
                  </button>
                  {onPreviewKit && (
                    <IconTip label={t("design.kit.preview", "预览套件")} side="left">
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        className="h-7 w-7 shrink-0 opacity-0 transition-opacity group-hover/sys:opacity-100"
                        onClick={(e) => {
                          e.stopPropagation()
                          onPreviewKit(s.id, s.name)
                        }}
                      >
                        <Eye className="h-4 w-4" />
                      </Button>
                    </IconTip>
                  )}
                </div>
              ))}
            </div>
          ))}
        </div>
        {footer && <div className="border-t p-2">{footer}</div>}
      </DialogContent>
    </Dialog>
  )
}
