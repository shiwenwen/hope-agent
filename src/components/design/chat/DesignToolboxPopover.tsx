import { useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { Search } from "lucide-react"

import { Input } from "@/components/ui/input"
import { cn } from "@/lib/utils"
import type { DesignRecipe } from "@/types/design"

interface Props {
  recipes: DesignRecipe[]
  /** 点选一个模板 → 用它生成一条起步 prompt 填入 composer（不自动发）。 */
  onPick: (prompt: string) => void
  /** 形态本地化标签（复用 DesignView 的 kindLabel；缺省用通用）。 */
  kindLabel?: (kind: string) => string
}

/**
 * 设计工具箱（B2-2）：把设计能力库（recipes/模板）做成可搜索、按形态分组的可浏览面板——
 * 此前只对模型可见（`list_recipes`），用户无法浏览/一键起步。点选 → 组合起步 prompt 填入
 * composer（不自动发，human-in-loop）。
 */
export function DesignToolboxPopover({ recipes, onPick, kindLabel }: Props) {
  const { t } = useTranslation()
  const [query, setQuery] = useState("")
  const label = kindLabel ?? ((k: string) => k)

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
    <div className="absolute right-0 top-full z-30 mt-1 w-[340px] rounded-xl border border-border/60 bg-popover/95 p-2 shadow-[0_8px_30px_rgb(0,0,0,0.12)] backdrop-blur-xl">
      <div className="relative mb-2">
        <Search className="pointer-events-none absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
        <Input
          autoFocus
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t("design.toolbox.search", "搜索模板 / 场景…")}
          className="h-8 pl-7 text-xs"
        />
      </div>
      {groups.length === 0 ? (
        <p className="py-4 text-center text-xs text-muted-foreground">
          {t("design.toolbox.empty", "没有匹配的模板")}
        </p>
      ) : (
        <div className="flex max-h-[340px] flex-col gap-1 overflow-y-auto">
          {groups.map(([kind, items]) => (
            <div key={kind}>
              <div className="px-1 py-1 text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                {label(kind)}
              </div>
              {items.map((r) => (
                <button
                  key={r.id}
                  type="button"
                  onClick={() => onPick(promptFor(r))}
                  className={cn(
                    "flex w-full flex-col gap-0.5 rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-secondary/60",
                  )}
                >
                  <span className="truncate text-xs font-medium">{r.name}</span>
                  {r.summary && (
                    <span className="truncate text-[11px] text-muted-foreground">{r.summary}</span>
                  )}
                </button>
              ))}
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

export default DesignToolboxPopover
