import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { Input } from "@/components/ui/input"
import { Search, X } from "lucide-react"
import { LEVEL_COLORS, LEVELS, CATEGORIES } from "./constants"

interface LogToolbarProps {
  filterLevels: string[]
  filterCategories: string[]
  keyword: string
  onToggleLevel: (level: string) => void
  onToggleCategory: (cat: string) => void
  onKeywordChange: (val: string) => void
  onClearAll: () => void
}

export default function LogToolbar({
  filterLevels,
  filterCategories,
  keyword,
  onToggleLevel,
  onToggleCategory,
  onKeywordChange,
  onClearAll,
}: LogToolbarProps) {
  const { t } = useTranslation()

  return (
    <div className="flex items-center gap-2 flex-wrap">
      {/* Level filter chips */}
      {LEVELS.map((level) => (
        <button
          key={level}
          onClick={() => onToggleLevel(level)}
          className={cn(
            "px-2 py-0.5 rounded-full text-xs font-medium transition-colors",
            filterLevels.includes(level)
              ? LEVEL_COLORS[level]
              : "bg-secondary/40 text-muted-foreground hover:bg-secondary/60",
          )}
        >
          {level}
        </button>
      ))}
      <span className="w-px h-4 bg-border" />
      {/* Category filter chips */}
      {CATEGORIES.map((cat) => (
        <button
          key={cat}
          onClick={() => onToggleCategory(cat)}
          className={cn(
            "px-2 py-0.5 rounded-full text-xs font-medium transition-colors",
            filterCategories.includes(cat)
              ? "bg-primary/10 text-primary"
              : "bg-secondary/40 text-muted-foreground hover:bg-secondary/60",
          )}
        >
          {cat}
        </button>
      ))}
      <span className="w-px h-4 bg-border" />
      {/* Keyword search */}
      <div className="relative flex-1 min-w-[160px] max-w-[300px]">
        <Search className="absolute left-2 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground" />
        <Input
          value={keyword}
          onChange={(e) => onKeywordChange(e.target.value)}
          placeholder={t("settings.logsSearch")}
          className="h-7 pl-7 pr-7 text-xs"
        />
        {keyword && (
          <button
            onClick={() => onKeywordChange("")}
            className="absolute right-2 top-1/2 -translate-y-1/2"
          >
            <X className="h-3 w-3 text-muted-foreground hover:text-foreground" />
          </button>
        )}
      </div>
      {(filterLevels.length > 0 || filterCategories.length > 0 || keyword) && (
        <button
          onClick={onClearAll}
          className="text-xs text-muted-foreground hover:text-foreground"
        >
          {t("settings.logsClearFilter")}
        </button>
      )}
    </div>
  )
}
