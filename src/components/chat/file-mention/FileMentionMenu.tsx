import { useEffect, useRef } from "react"
import { useTranslation } from "react-i18next"
import { File, Folder, Loader2 } from "lucide-react"
import { cn } from "@/lib/utils"
import type { MentionEntry, MentionMode } from "./types"

interface FileMentionMenuProps {
  isOpen: boolean
  entries: MentionEntry[]
  selectedIndex: number
  mode: MentionMode
  /** Absolute path of the directory being shown (list mode) — surfaced as breadcrumb. */
  dirPath: string | null
  workingDir: string | null
  loading: boolean
  error: string | null
  truncated: boolean
  onSelect: (entry: MentionEntry) => void
  onHover: (index: number) => void
}

export default function FileMentionMenu({
  isOpen,
  entries,
  selectedIndex,
  mode,
  dirPath,
  workingDir,
  loading,
  error,
  truncated,
  onSelect,
  onHover,
}: FileMentionMenuProps) {
  const { t } = useTranslation()
  const selectedRef = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    selectedRef.current?.scrollIntoView({ block: "nearest" })
  }, [selectedIndex])

  if (!isOpen) return null

  // Compute breadcrumb relative to workingDir for list mode; search mode shows
  // the working dir basename.
  const breadcrumb = computeBreadcrumb(workingDir, dirPath, mode)

  return (
    <div
      className="absolute bottom-full left-0 right-0 mb-2 mx-3 z-50 max-h-[320px] overflow-y-auto overscroll-contain
                 bg-popover/95 backdrop-blur-xl border border-border/60 rounded-xl
                 shadow-[0_8px_30px_rgb(0,0,0,0.12)] p-1.5
                 animate-in fade-in-0 zoom-in-95 slide-in-from-bottom-1 duration-150"
      role="listbox"
    >
      <div className="flex items-center gap-2 px-2.5 py-1 text-[11px] font-medium text-muted-foreground/70 uppercase tracking-wider">
        <span className="truncate">
          {mode === "search"
            ? t("chat.fileMention.searchHeader")
            : t("chat.fileMention.breadcrumb", { path: breadcrumb || "/" })}
        </span>
        {loading && <Loader2 className="h-3 w-3 animate-spin" />}
        {truncated && (
          <span className="ml-auto text-[10px] text-amber-500/80 normal-case tracking-normal">
            {t("chat.fileMention.truncated")}
          </span>
        )}
      </div>

      {error && (
        <div className="px-2.5 py-2 text-[12px] text-destructive">{error}</div>
      )}

      {!loading && !error && entries.length === 0 && (
        <div className="px-2.5 py-2 text-[12px] text-muted-foreground/70">
          {t("chat.fileMention.empty")}
        </div>
      )}

      {entries.map((entry, idx) => {
        const isSelected = idx === selectedIndex
        return (
          <button
            key={`${entry.path}-${idx}`}
            ref={isSelected ? selectedRef : undefined}
            type="button"
            role="option"
            aria-selected={isSelected}
            className={cn(
              "w-full text-left px-2.5 py-1.5 rounded-md transition-all duration-100 flex items-center gap-2 outline-none",
              isSelected
                ? "bg-secondary text-foreground shadow-sm"
                : "text-foreground/80 hover:bg-secondary/60 hover:text-foreground",
            )}
            onClick={() => onSelect(entry)}
            onMouseEnter={() => onHover(idx)}
          >
            {entry.isDir ? (
              <Folder className="h-3.5 w-3.5 shrink-0 text-primary/70" />
            ) : (
              <File className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
            )}
            <span className="font-mono text-[13px] truncate">
              {entry.name}
              {entry.isDir ? "/" : ""}
            </span>
            {mode === "search" && entry.relPath !== entry.name && (
              <span className="ml-auto truncate text-[11px] text-muted-foreground/60 font-mono">
                {entry.relPath}
              </span>
            )}
          </button>
        )
      })}
    </div>
  )
}

function computeBreadcrumb(
  workingDir: string | null,
  dirPath: string | null,
  mode: MentionMode,
): string {
  if (!dirPath) return ""
  if (mode === "search") {
    if (!workingDir) return dirPath
    const parts = workingDir.split("/").filter(Boolean)
    return parts[parts.length - 1] ?? workingDir
  }
  if (!workingDir || !dirPath.startsWith(workingDir)) return dirPath
  const rel = dirPath.slice(workingDir.length).replace(/^\//, "")
  return rel || ""
}
