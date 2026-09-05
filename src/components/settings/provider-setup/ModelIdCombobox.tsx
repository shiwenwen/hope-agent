import { useEffect, useId, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Popover, PopoverAnchor, PopoverContent } from "@/components/ui/popover"
import { cn } from "@/lib/utils"
import { MODEL_CATALOG, searchModelCatalog, type ModelCatalogEntry } from "./model-catalog"

interface ModelIdComboboxProps {
  value: string
  onChange: (value: string) => void
  onSelect: (entry: ModelCatalogEntry) => void
}

function formatTokens(value: number) {
  return new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 }).format(value)
}

export function ModelIdCombobox({ value, onChange, onSelect }: ModelIdComboboxProps) {
  const { t } = useTranslation()
  const listboxId = useId()
  const inputRef = useRef<HTMLInputElement>(null)
  const optionRefs = useRef<Array<HTMLButtonElement | null>>([])
  const [open, setOpen] = useState(false)
  const [activeIndex, setActiveIndex] = useState(0)
  const suggestions = useMemo(() => searchModelCatalog(MODEL_CATALOG, value), [value])
  const visible = open && suggestions.length > 0
  const selectedIndex = Math.min(activeIndex, Math.max(suggestions.length - 1, 0))

  useEffect(() => {
    if (!visible) return
    optionRefs.current[selectedIndex]?.scrollIntoView({ block: "nearest" })
  }, [selectedIndex, suggestions, visible])

  function select(entry: ModelCatalogEntry) {
    onSelect(entry)
    setOpen(false)
    setActiveIndex(0)
  }

  return (
    <Popover open={visible} onOpenChange={setOpen}>
      <PopoverAnchor asChild>
        <Input
          ref={inputRef}
          value={value}
          autoCorrect="off"
          autoCapitalize="none"
          autoComplete="off"
          spellCheck={false}
          role="combobox"
          aria-autocomplete="list"
          aria-controls={listboxId}
          aria-expanded={visible}
          aria-activedescendant={visible ? `${listboxId}-option-${selectedIndex}` : undefined}
          onFocus={() => setOpen(true)}
          onChange={(event) => {
            onChange(event.target.value)
            setActiveIndex(0)
            setOpen(true)
          }}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              setOpen(false)
              return
            }
            if (!suggestions.length) return
            if (event.key === "ArrowDown") {
              event.preventDefault()
              setOpen(true)
              setActiveIndex((index) => (index + 1) % suggestions.length)
              return
            }
            if (event.key === "ArrowUp") {
              event.preventDefault()
              setOpen(true)
              setActiveIndex((index) => (index - 1 + suggestions.length) % suggestions.length)
              return
            }
            if (event.key === "Enter" && visible) {
              event.preventDefault()
              select(suggestions[selectedIndex])
            }
          }}
          placeholder="model-id"
          className="h-8 text-xs"
        />
      </PopoverAnchor>
      <PopoverContent
        align="start"
        sideOffset={4}
        onOpenAutoFocus={(event) => event.preventDefault()}
        onCloseAutoFocus={(event) => event.preventDefault()}
        onPointerDownOutside={(event) => {
          if (inputRef.current?.contains(event.target as Node)) event.preventDefault()
        }}
        className="w-[var(--radix-popover-trigger-width)] min-w-[22rem] max-w-[calc(100vw-2rem)] p-1"
      >
        <div id={listboxId} role="listbox" className="max-h-72 overflow-y-auto">
          {suggestions.map((entry, index) => (
            <Button
              key={entry.catalogKey}
              ref={(node) => {
                optionRefs.current[index] = node
              }}
              id={`${listboxId}-option-${index}`}
              type="button"
              role="option"
              aria-selected={index === selectedIndex}
              variant="ghost"
              onMouseEnter={() => setActiveIndex(index)}
              onClick={() => select(entry)}
              className={cn(
                "h-auto w-full justify-start rounded-md px-2 py-2 text-left font-normal",
                index === selectedIndex && "bg-secondary",
              )}
            >
              <span className="min-w-0 flex-1">
                <span className="flex min-w-0 items-baseline gap-2">
                  <span className="truncate text-xs font-medium text-foreground">{entry.name}</span>
                  <span className="truncate font-mono text-[10px] text-muted-foreground">
                    {entry.id}
                  </span>
                </span>
                <span className="mt-0.5 block truncate text-[10px] text-muted-foreground/70">
                  {entry.sourceNames.join(" · ")}
                </span>
              </span>
              <span className="ml-3 shrink-0 text-right text-[9px] leading-4 text-muted-foreground/70">
                <span className="block">
                  {t("model.contextWindow")}: {formatTokens(entry.contextWindow)}
                </span>
                <span className="block">
                  {t("model.maxTokens")}: {formatTokens(entry.maxTokens)}
                </span>
              </span>
            </Button>
          ))}
        </div>
      </PopoverContent>
    </Popover>
  )
}
