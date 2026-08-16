/**
 * Clickable path breadcrumb for the preview header. Directory segments jump to
 * that folder in the Files panel (only when the host can resolve them, so a
 * segment is never a dead click); the trailing file segment copies the path.
 */

import { Fragment, useCallback, useMemo } from "react"
import { useTranslation } from "react-i18next"
import { ChevronRight } from "lucide-react"
import { toast } from "sonner"

import { cn } from "@/lib/utils"
import { splitPathSegments } from "./filePathSegments"

interface FilePathBreadcrumbProps {
  path: string
  className?: string
  /** Reveal a directory segment elsewhere in the UI (the Files panel). */
  onNavigateDirectory?: (dirPath: string) => void
  /** Gate per segment: unresolvable directories render as plain text. */
  canNavigateDirectory?: (dirPath: string) => boolean
}

export function FilePathBreadcrumb({
  path,
  className,
  onNavigateDirectory,
  canNavigateDirectory,
}: FilePathBreadcrumbProps) {
  const { t } = useTranslation()
  const segments = useMemo(() => splitPathSegments(path), [path])

  const copyPath = useCallback(() => {
    void navigator.clipboard.writeText(path).then(
      () => toast.success(t("fileBrowser.copied", "Copied")),
      () => toast.error(t("fileBrowser.copyFailed", "Copy failed")),
    )
  }, [path, t])

  if (segments.length === 0) return null

  return (
    <nav
      aria-label={t("filePreview.pathBreadcrumb", "File path")}
      className={cn(
        "flex min-w-0 items-center font-mono text-[11px] leading-tight text-muted-foreground",
        className,
      )}
    >
      {segments.map((segment, index) => {
        const navigable =
          segment.isDir && !!onNavigateDirectory && (canNavigateDirectory?.(segment.path) ?? true)
        return (
          <Fragment key={segment.path}>
            {index > 0 && (
              <ChevronRight className="h-3 w-3 shrink-0 opacity-50" aria-hidden="true" />
            )}
            {navigable || !segment.isDir ? (
              <button
                type="button"
                className={cn(
                  "min-w-0 truncate rounded px-0.5 transition-colors hover:bg-secondary/60 hover:text-foreground",
                  !segment.isDir && "shrink-0 text-foreground/70",
                )}
                data-ha-title-tip={
                  segment.isDir
                    ? t("filePreview.revealFolder", "Show folder in Files")
                    : t("filePreview.copyPath", "Copy full path")
                }
                onClick={() => (segment.isDir ? onNavigateDirectory?.(segment.path) : copyPath())}
              >
                {segment.label}
              </button>
            ) : (
              <span className="min-w-0 truncate px-0.5">{segment.label}</span>
            )}
          </Fragment>
        )
      })}
    </nav>
  )
}
