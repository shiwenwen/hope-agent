import { useEffect, useRef } from "react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import type { SlashCommandDef, CommandCategory } from "./types"
import { CATEGORY_ORDER } from "./types"

interface SlashCommandMenuProps {
  commands: SlashCommandDef[]
  selectedIndex: number
  onSelect: (cmd: SlashCommandDef) => void
}

const CATEGORY_I18N_KEYS: Record<CommandCategory, string> = {
  session: "slashCommands.categories.session",
  model: "slashCommands.categories.model",
  memory: "slashCommands.categories.memory",
  agent: "slashCommands.categories.agent",
  utility: "slashCommands.categories.utility",
  skill: "slashCommands.categories.skill",
}

export default function SlashCommandMenu({
  commands,
  selectedIndex,
  onSelect,
}: SlashCommandMenuProps) {
  const { t } = useTranslation()
  const menuRef = useRef<HTMLDivElement>(null)
  const selectedRef = useRef<HTMLButtonElement>(null)

  // Scroll selected item into view
  useEffect(() => {
    selectedRef.current?.scrollIntoView({ block: "nearest" })
  }, [selectedIndex])

  if (commands.length === 0) return null

  // Group by category
  const grouped = new Map<CommandCategory, SlashCommandDef[]>()
  for (const cmd of commands) {
    const list = grouped.get(cmd.category) || []
    list.push(cmd)
    grouped.set(cmd.category, list)
  }

  let flatIndex = 0

  return (
    <div
      ref={menuRef}
      className="absolute bottom-full left-0 right-0 mb-2 mx-3 bg-popover/95 backdrop-blur-xl border border-border/60 rounded-xl shadow-[0_8px_30px_rgb(0,0,0,0.12)] z-50 max-h-[300px] overflow-y-auto overscroll-contain p-1.5 animate-in fade-in-0 zoom-in-95 slide-in-from-bottom-1 duration-150"
    >
      {CATEGORY_ORDER.filter((cat) => grouped.has(cat)).map((cat) => {
        const cmds = grouped.get(cat)!
        return (
          <div key={cat}>
            <div className="px-2.5 py-1 text-[11px] font-medium text-muted-foreground/60 uppercase tracking-wider">
              {t(CATEGORY_I18N_KEYS[cat])}
            </div>
            {cmds.map((cmd) => {
              const idx = flatIndex++
              const isSelected = idx === selectedIndex
              return (
                <button
                  key={cmd.name}
                  ref={isSelected ? selectedRef : undefined}
                  className={cn(
                    "w-full text-left px-2.5 py-1.5 rounded-md transition-all duration-100 flex items-center gap-2",
                    isSelected
                      ? "bg-secondary text-foreground shadow-sm"
                      : "text-foreground/80 hover:bg-secondary/60 hover:text-foreground",
                  )}
                  onClick={() => onSelect(cmd)}
                  onMouseEnter={(e) => {
                    // Let mouse hovering also highlight
                    e.currentTarget.focus()
                  }}
                >
                  <span className="font-mono text-[13px] text-primary/80 shrink-0">
                    /{cmd.name}
                  </span>
                  <span className="text-[12px] text-muted-foreground truncate">
                    {cmd.descriptionRaw || t(cmd.descriptionKey)}
                  </span>
                  {cmd.argPlaceholder && (
                    <span className="text-[11px] text-muted-foreground/50 ml-auto shrink-0">
                      {cmd.argPlaceholder}
                    </span>
                  )}
                </button>
              )
            })}
          </div>
        )
      })}
    </div>
  )
}
