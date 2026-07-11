import { useEffect, useRef } from "react"
import { useTranslation } from "react-i18next"
import { CircleAlert, Loader2 } from "lucide-react"
import { cn } from "@/lib/utils"
import type { ReferenceableNote } from "@/types/knowledge"

interface Props {
  isOpen: boolean
  entries: ReferenceableNote[]
  selectedIndex: number
  loading: boolean
  loadErrorDetail: string | null
  onSelect: (entry: ReferenceableNote) => void
  onHover: (index: number) => void
}

/**
 * Popper for the `[[note]]` picker, modeled on FileMentionMenu. Flat list across
 * all reachable KBs; each row shows the KB emoji, note title, and KB name.
 */
export default function NoteMentionMenu({
  isOpen,
  entries,
  selectedIndex,
  loading,
  loadErrorDetail,
  onSelect,
  onHover,
}: Props) {
  const { t } = useTranslation()
  const selectedRef = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    selectedRef.current?.scrollIntoView({ block: "nearest" })
  }, [selectedIndex])

  if (!isOpen) return null

  return (
    <div
      className="absolute bottom-full left-0 mb-2 w-[360px] max-h-[320px] overflow-y-auto bg-popover/95 backdrop-blur-xl border border-border/60 rounded-xl shadow-[0_8px_30px_rgb(0,0,0,0.12)] z-50 p-1.5 animate-in fade-in-0 zoom-in-95 slide-in-from-bottom-1 duration-150"
      role="listbox"
    >
      <div className="flex items-center justify-between px-2 py-1 text-[11px] font-medium text-muted-foreground">
        <span>{t("knowledge.mention.heading", "Knowledge notes")}</span>
        {loading && <Loader2 className="h-3 w-3 animate-spin" />}
      </div>

      {loadErrorDetail ? (
        <div className="mx-1 mb-1 flex gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-2 py-1.5 text-[11px] leading-relaxed text-amber-800 dark:text-amber-200">
          <CircleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <div className="min-w-0">
            <div className="font-medium">
              {t("knowledge.mention.loadFailed", "Failed to load knowledge notes")}
            </div>
            <div className="mt-0.5 break-words opacity-85">
              {t("knowledge.mention.errorDetail", "Details: {{error}}", {
                error: loadErrorDetail,
              })}
            </div>
          </div>
        </div>
      ) : entries.length === 0 && !loading ? (
        <p className="px-2 py-2 text-xs text-muted-foreground">
          {t("knowledge.mention.empty", "No notes — attach a knowledge space first.")}
        </p>
      ) : (
        entries.map((n, idx) => (
          <button
            key={`${n.kbId}:${n.relPath}`}
            ref={idx === selectedIndex ? selectedRef : undefined}
            type="button"
            role="option"
            aria-selected={idx === selectedIndex}
            onClick={() => onSelect(n)}
            onMouseEnter={() => onHover(idx)}
            className={cn(
              "flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-[13px] transition-colors",
              idx === selectedIndex
                ? "bg-secondary/70 text-foreground"
                : "text-foreground/80 hover:bg-secondary/40",
            )}
          >
            <span className="shrink-0 text-sm leading-none">{n.kbEmoji || "📓"}</span>
            <span className="min-w-0 flex-1 truncate">{n.title}</span>
            <span className="shrink-0 max-w-[45%] truncate text-[10px] text-muted-foreground">
              {n.kbName}
            </span>
          </button>
        ))
      )}
    </div>
  )
}
