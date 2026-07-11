import { useEffect, useRef } from "react"
import { useTranslation } from "react-i18next"
import { Hash } from "lucide-react"
import { FloatingMenu } from "@/components/ui/floating-menu"
import { cn } from "@/lib/utils"
import type { QuickPromptItem } from "@/types/quickPrompts"

interface QuickPromptMenuProps {
  isOpen: boolean
  entries: QuickPromptItem[]
  selectedIndex: number
  query: string
  onSelect: (entry: QuickPromptItem) => void
  onHover: (index: number) => void
}

function previewText(content: string): string {
  const compact = content.replace(/\s+/g, " ").trim()
  return compact.length > 140 ? `${compact.slice(0, 140)}...` : compact
}

export default function QuickPromptMenu({
  isOpen,
  entries,
  selectedIndex,
  query,
  onSelect,
  onHover,
}: QuickPromptMenuProps) {
  const { t } = useTranslation()
  const selectedRef = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    selectedRef.current?.scrollIntoView({ block: "nearest" })
  }, [selectedIndex])

  const sectionHeaderClass =
    "flex items-center gap-2 px-2.5 py-1 text-[11px] font-medium text-muted-foreground/70 uppercase tracking-wider"
  const rowClass = (selected: boolean) =>
    cn(
      "w-full text-left px-2.5 py-1.5 rounded-md transition-all duration-100 flex min-w-0 items-start gap-2 outline-none",
      selected
        ? "bg-secondary text-foreground shadow-sm"
        : "text-foreground/80 hover:bg-secondary/60 hover:text-foreground",
    )

  return (
    <FloatingMenu
      open={isOpen}
      positionClassName="bottom-full left-0 right-0 mb-2 mx-3"
      className="max-h-[320px] overflow-y-auto overscroll-contain p-1.5"
      role="listbox"
    >
      <div className={sectionHeaderClass}>
        <Hash className="h-3 w-3" />
        <span className="truncate">{t("chat.quickPrompts.heading")}</span>
      </div>

      {entries.length === 0 ? (
        <div className="px-2.5 py-2 text-[12px] text-muted-foreground/70">
          {query.trim()
            ? t("chat.quickPrompts.noMatches")
            : t("chat.quickPrompts.empty")}
        </div>
      ) : (
        entries.map((entry, index) => {
          const isSelected = index === selectedIndex
          return (
            <button
              key={entry.id}
              ref={isSelected ? selectedRef : undefined}
              type="button"
              role="option"
              aria-selected={isSelected}
              className={rowClass(isSelected)}
              onClick={() => onSelect(entry)}
              onMouseEnter={() => onHover(index)}
              title={entry.content}
            >
              <Hash className="mt-0.5 h-3.5 w-3.5 shrink-0 text-primary/70" />
              <span className="min-w-0 flex-1">
                <span className="block truncate text-[13px] font-medium">{entry.title}</span>
                <span className="mt-0.5 block truncate text-[11px] text-muted-foreground/65">
                  {previewText(entry.content)}
                </span>
              </span>
            </button>
          )
        })
      )}
    </FloatingMenu>
  )
}
