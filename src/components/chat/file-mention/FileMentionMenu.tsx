import { useEffect, useRef } from "react"
import { useTranslation } from "react-i18next"
import { File, FileText, Folder, Loader2 } from "lucide-react"
import { cn } from "@/lib/utils"
import { FloatingMenu } from "@/components/ui/floating-menu"
import { SkillMentionIcon } from "../skill-mention/SkillMentionIcon"
import { skillMentionMeta, type MentionableSkill } from "../skill-mention/skillTokens"
import type { MentionEntry, MentionMode } from "./types"
import type { ReferenceableNote } from "@/types/knowledge"

interface FileMentionMenuProps {
  isOpen: boolean
  entries: MentionEntry[]
  /** Knowledge-note section rows (already filtered). */
  noteEntries: ReferenceableNote[]
  /** Knowledge-note fetch in flight. */
  notesLoading: boolean
  /** Whether a note source exists (drives the note header / empty state). */
  noteCapable: boolean
  /** Built-in skill section rows (already filtered). */
  skillEntries: MentionableSkill[]
  /** Whether the skill section is enabled (drives its header). */
  skillCapable: boolean
  /** Flat cursor over `[...entries, ...noteEntries, ...skillEntries]`. */
  selectedIndex: number
  mode: MentionMode
  /** Absolute path of the directory being shown (list mode) — surfaced as breadcrumb. */
  dirPath: string | null
  workingDir: string | null
  loading: boolean
  error: string | null
  truncated: boolean
  /** File rows stay hidden for a bare `@`; other mentionable types can still show. */
  hasFileQuery: boolean
  onSelect: (entry: MentionEntry) => void
  onSelectNote: (note: ReferenceableNote) => void
  onSelectSkill: (skill: MentionableSkill) => void
  /** Hover handler; receives the FLAT index across all sections. */
  onHover: (index: number) => void
}

export default function FileMentionMenu({
  isOpen,
  entries,
  noteEntries,
  notesLoading,
  noteCapable,
  skillEntries,
  skillCapable,
  selectedIndex,
  mode,
  dirPath,
  workingDir,
  loading,
  error,
  truncated,
  hasFileQuery,
  onSelect,
  onSelectNote,
  onSelectSkill,
  onHover,
}: FileMentionMenuProps) {
  const { t } = useTranslation()
  const selectedRef = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    selectedRef.current?.scrollIntoView({ block: "nearest" })
  }, [selectedIndex])

  if (!isOpen) return null

  const hasFiles = entries.length > 0
  const hasNotes = noteEntries.length > 0
  const hasSkills = skillEntries.length > 0
  const showFileSection = !!workingDir && hasFileQuery
  // Nothing to paint: no file section (working dir / its loading+empty/error),
  // no note rows or in-flight note load, and no skill rows. Avoids an empty
  // floating box when `@` opens with nothing to show.
  if (!showFileSection && !error && !hasNotes && !notesLoading && !hasSkills) return null

  // Compute breadcrumb relative to workingDir for list mode; search mode shows
  // the working dir basename.
  const breadcrumb = computeBreadcrumb(workingDir, dirPath, mode)
  const showNoteSection = hasNotes || (noteCapable && notesLoading)
  const showSkillSection = skillCapable && hasSkills
  const sectionHeaderClass =
    "flex items-center gap-2 px-2.5 py-1 text-[11px] font-medium text-muted-foreground/70 uppercase tracking-wider"
  const rowClass = (selected: boolean) =>
    cn(
      "w-full text-left px-2.5 py-1.5 rounded-md transition-all duration-100 flex items-center gap-2 outline-none",
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
      {/* ── Files section (only when a working dir is set) ── */}
      {showFileSection && (
        <div className={sectionHeaderClass}>
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
      )}

      {error && <div className="px-2.5 py-2 text-[12px] text-destructive">{error}</div>}

      {showFileSection && !loading && !error && !hasFiles && (
        <div className="px-2.5 py-2 text-[12px] text-muted-foreground/70">
          {t("chat.fileMention.empty")}
        </div>
      )}

      {showFileSection &&
        entries.map((entry, idx) => {
          const isSelected = idx === selectedIndex
          return (
            <button
              key={`file-${entry.path}-${idx}`}
              ref={isSelected ? selectedRef : undefined}
              type="button"
              role="option"
              aria-selected={isSelected}
              className={rowClass(isSelected)}
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

      {/* ── Knowledge-notes section ── */}
      {showNoteSection && (
        <div
          className={cn(
            sectionHeaderClass,
            showFileSection && hasFiles && "mt-1 border-t border-border/40 pt-1.5",
          )}
        >
          <span className="truncate normal-case tracking-normal">
            {t("knowledge.mention.heading", "Knowledge notes")}
          </span>
          {notesLoading && <Loader2 className="h-3 w-3 animate-spin" />}
        </div>
      )}

      {noteEntries.map((note, j) => {
        const flatIdx = entries.length + j
        const isSelected = flatIdx === selectedIndex
        return (
          <button
            key={`note-${note.kbId}-${note.relPath}`}
            ref={isSelected ? selectedRef : undefined}
            type="button"
            role="option"
            aria-selected={isSelected}
            className={rowClass(isSelected)}
            onClick={() => onSelectNote(note)}
            onMouseEnter={() => onHover(flatIdx)}
          >
            {note.kbEmoji ? (
              <span className="shrink-0 text-sm leading-none">{note.kbEmoji}</span>
            ) : (
              <FileText className="h-4 w-4 shrink-0 text-violet-500 dark:text-violet-400" />
            )}
            <span className="truncate text-[13px] text-foreground">{note.title}</span>
            <span className="ml-auto max-w-[40%] truncate text-[11px] text-muted-foreground/60">
              {note.kbName}
            </span>
          </button>
        )
      })}

      {/* ── Built-in skills section (`@skill:<name>`) ── */}
      {showSkillSection && (
        <div
          className={cn(
            sectionHeaderClass,
            ((showFileSection && hasFiles) || showNoteSection) &&
              "mt-1 border-t border-border/40 pt-1.5",
          )}
        >
          <span className="truncate normal-case tracking-normal">
            {t("chat.skillMention.heading", "Skills")}
          </span>
        </div>
      )}

      {skillEntries.map((skill, k) => {
        const flatIdx = entries.length + noteEntries.length + k
        const isSelected = flatIdx === selectedIndex
        const meta = skillMentionMeta(skill.name)
        const label = meta ? t(meta.labelKey) : skill.name
        return (
          <button
            key={`skill-${skill.name}`}
            ref={isSelected ? selectedRef : undefined}
            type="button"
            role="option"
            aria-selected={isSelected}
            className={rowClass(isSelected)}
            onClick={() => onSelectSkill(skill)}
            onMouseEnter={() => onHover(flatIdx)}
            title={skill.description}
          >
            {meta && (
              <SkillMentionIcon kind={meta.iconKind} className="h-4 w-4 shrink-0 text-rose-500" />
            )}
            <span className="text-[13px] truncate">{label}</span>
            <span className="ml-auto shrink-0 font-mono text-[11px] text-muted-foreground/50">
              @skill
            </span>
          </button>
        )
      })}
    </FloatingMenu>
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
