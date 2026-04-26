import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { Input } from "@/components/ui/input"
import { Button } from "@/components/ui/button"
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
        <Button
          key={level}
          variant="ghost"
          size="sm"
          onClick={() => onToggleLevel(level)}
          className={cn(
            "h-auto rounded-full px-2 py-0.5 text-xs font-medium",
            filterLevels.includes(level)
              ? LEVEL_COLORS[level]
              : "bg-secondary/40 text-muted-foreground hover:bg-secondary/60",
          )}
        >
          {level}
        </Button>
      ))}
      <span className="w-px h-4 bg-border" />
      {/* Category filter chips */}
      {CATEGORIES.map((cat) => (
        <Button
          key={cat}
          variant="ghost"
          size="sm"
          onClick={() => onToggleCategory(cat)}
          className={cn(
            "h-auto rounded-full px-2 py-0.5 text-xs font-medium",
            filterCategories.includes(cat)
              ? "bg-primary/10 text-primary hover:bg-primary/15 hover:text-primary"
              : "bg-secondary/40 text-muted-foreground hover:bg-secondary/60",
          )}
        >
          {cat}
        </Button>
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
          <Button
            variant="ghost"
            size="icon"
            onClick={() => onKeywordChange("")}
            className="absolute right-1 top-1/2 -translate-y-1/2 h-5 w-5 text-muted-foreground hover:text-foreground"
          >
            <X className="h-3 w-3" />
          </Button>
        )}
      </div>
      {(filterLevels.length > 0 || filterCategories.length > 0 || keyword) && (
        <Button
          variant="ghost"
          size="sm"
          onClick={onClearAll}
          className="h-auto px-2 py-1 text-xs font-normal text-muted-foreground hover:bg-transparent hover:text-foreground"
        >
          {t("settings.logsClearFilter")}
        </Button>
      )}
    </div>
  )
}
