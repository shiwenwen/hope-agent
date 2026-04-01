import { useEffect, useRef } from "react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { ChevronRight } from "lucide-react"
import type { SlashCommandDef, CommandCategory } from "./types"
import { CATEGORY_ORDER } from "./types"

interface SlashCommandMenuProps {
  commands: SlashCommandDef[]
  selectedIndex: number
  onSelect: (cmd: SlashCommandDef) => void
  /** Currently expanded command (showing arg options submenu) */
  expandedCmd?: SlashCommandDef | null
  /** Filtered options for the expanded command */
  filteredOptions?: string[]
  /** Selected option index in the submenu */
  selectedOptionIndex?: number
  /** Execute a specific option from the submenu */
  onSelectOption?: (cmd: SlashCommandDef, option: string) => void
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
  expandedCmd,
  filteredOptions = [],
  selectedOptionIndex = 0,
  onSelectOption,
}: SlashCommandMenuProps) {
  const { t } = useTranslation()
  const menuRef = useRef<HTMLDivElement>(null)
  const selectedRef = useRef<HTMLButtonElement>(null)
  const selectedOptionRef = useRef<HTMLButtonElement>(null)

  // Scroll selected item into view
  useEffect(() => {
    if (expandedCmd) {
      selectedOptionRef.current?.scrollIntoView({ block: "nearest" })
    } else {
      selectedRef.current?.scrollIntoView({ block: "nearest" })
    }
  }, [selectedIndex, selectedOptionIndex, expandedCmd])

  // When showing submenu only (auto-expanded via input), render just the options
  if (expandedCmd && commands.length === 0) {
    return (
      <div
        ref={menuRef}
        className="absolute bottom-full left-0 right-0 mb-2 mx-3 bg-popover/95 backdrop-blur-xl border border-border/60 rounded-xl shadow-[0_8px_30px_rgb(0,0,0,0.12)] z-50 max-h-[300px] overflow-y-auto overscroll-contain p-1.5 animate-in fade-in-0 zoom-in-95 slide-in-from-bottom-1 duration-150"
      >
        <div className="px-2.5 py-1 text-[11px] font-medium text-muted-foreground/60 uppercase tracking-wider">
          /{expandedCmd.name}
        </div>
        {renderOptions(expandedCmd, filteredOptions, selectedOptionIndex, selectedOptionRef, onSelectOption)}
      </div>
    )
  }

  if (commands.length === 0 && !expandedCmd) return null

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
              const isSelected = idx === selectedIndex && !expandedCmd
              const isExpanded = expandedCmd?.name === cmd.name
              return (
                <div key={cmd.name}>
                  <button
                    ref={isSelected ? selectedRef : undefined}
                    className={cn(
                      "w-full text-left px-2.5 py-1.5 rounded-md transition-all duration-100 flex items-center gap-2",
                      isSelected
                        ? "bg-secondary text-foreground shadow-sm"
                        : isExpanded
                          ? "bg-secondary/40 text-foreground"
                          : "text-foreground/80 hover:bg-secondary/60 hover:text-foreground",
                    )}
                    onClick={() => onSelect(cmd)}
                    onMouseEnter={(e) => {
                      e.currentTarget.focus()
                    }}
                  >
                    <span className="font-mono text-[13px] text-primary/80 shrink-0">
                      /{cmd.name}
                    </span>
                    <span className="text-[12px] text-muted-foreground truncate">
                      {cmd.descriptionRaw || t(cmd.descriptionKey)}
                    </span>
                    {cmd.argOptions?.length ? (
                      <ChevronRight
                        className={cn(
                          "w-3 h-3 ml-auto shrink-0 transition-transform",
                          isExpanded ? "rotate-90 text-primary/60" : "text-muted-foreground/40",
                        )}
                      />
                    ) : cmd.argPlaceholder ? (
                      <span className="text-[11px] text-muted-foreground/50 ml-auto shrink-0">
                        {cmd.argPlaceholder}
                      </span>
                    ) : null}
                  </button>
                  {/* Inline submenu for arg options */}
                  {isExpanded && filteredOptions.length > 0 && (
                    <div className="ml-5 my-0.5 border-l-2 border-border/40 pl-1.5">
                      {renderOptions(cmd, filteredOptions, selectedOptionIndex, selectedOptionRef, onSelectOption)}
                    </div>
                  )}
                </div>
              )
            })}
          </div>
        )
      })}
    </div>
  )
}

/** Render option buttons for a command's argOptions submenu */
function renderOptions(
  cmd: SlashCommandDef,
  options: string[],
  selectedIdx: number,
  selectedRef: React.RefObject<HTMLButtonElement | null>,
  onSelectOption?: (cmd: SlashCommandDef, option: string) => void,
) {
  return options.map((opt, i) => (
    <button
      key={opt}
      ref={i === selectedIdx ? selectedRef : undefined}
      className={cn(
        "w-full text-left px-2.5 py-1 rounded-md text-[13px] font-mono transition-all duration-100",
        i === selectedIdx
          ? "bg-primary/10 text-primary"
          : "text-foreground/70 hover:bg-secondary/50 hover:text-foreground",
      )}
      onClick={() => onSelectOption?.(cmd, opt)}
    >
      {opt}
    </button>
  ))
}
