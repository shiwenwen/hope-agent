import { useState } from "react"
import { FolderCheck, FolderPlus, Loader2, X } from "lucide-react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { IconTip } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import { getTransport, isTauriMode } from "@/lib/transport"
import { logger } from "@/lib/logger"
import ServerDirectoryBrowser from "./ServerDirectoryBrowser"

interface WorkingDirectoryButtonProps {
  sessionId: string | null
  workingDir: string | null | undefined
  saving?: boolean
  disabled?: boolean
  /**
   * Fired with the canonical path (or `null` to clear). Parent is
   * responsible for persisting to the backend.
   */
  onChange: (workingDir: string | null) => void
}

function basename(path: string): string {
  const normalized = path.replace(/[\\/]+$/, "")
  const parts = normalized.split(/[\\/]/).filter(Boolean)
  return parts.length > 0 ? parts[parts.length - 1] : normalized || path
}

export default function WorkingDirectoryButton({
  sessionId,
  workingDir,
  saving = false,
  disabled = false,
  onChange,
}: WorkingDirectoryButtonProps) {
  const { t } = useTranslation()
  const [browserOpen, setBrowserOpen] = useState(false)
  const hasSelection = typeof workingDir === "string" && workingDir.length > 0

  const handlePick = async () => {
    if (disabled || saving) return
    if (isTauriMode()) {
      try {
        const picked = await getTransport().pickLocalDirectory()
        if (!picked) return
        onChange(picked)
      } catch (e) {
        logger.error(
          "chat",
          "WorkingDirectoryButton::pickLocalDirectory",
          "native directory picker failed",
          e,
        )
        toast.error(t("chat.workingDir.invalid"), {
          description: e instanceof Error ? e.message : String(e),
        })
      }
    } else {
      setBrowserOpen(true)
    }
  }

  const handleClear = (e: React.MouseEvent) => {
    e.stopPropagation()
    if (disabled || saving) return
    onChange(null)
  }

  const tooltipLabel = hasSelection
    ? `${t("chat.workingDir.current")}: ${workingDir}`
    : sessionId
      ? t("chat.workingDir.select")
      : t("chat.workingDir.selectPreset")

  const label = hasSelection ? basename(workingDir!) : t("chat.workingDir.select")

  return (
    <>
      <IconTip label={tooltipLabel}>
        <button
          type="button"
          disabled={saving || disabled}
          onClick={handlePick}
          className={cn(
            "flex items-center gap-1 bg-transparent text-xs font-medium px-2 py-1 rounded-lg cursor-pointer transition-colors hover:bg-secondary shrink-0 whitespace-nowrap max-w-[200px] disabled:cursor-not-allowed disabled:opacity-50",
            saving && "disabled:cursor-wait disabled:opacity-70",
            hasSelection
              ? "text-primary hover:text-primary"
              : "text-muted-foreground hover:text-foreground",
          )}
        >
          {saving ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin shrink-0" />
          ) : hasSelection ? (
            <FolderCheck className="h-3.5 w-3.5 shrink-0" />
          ) : (
            <FolderPlus className="h-3.5 w-3.5 shrink-0" />
          )}
          <span className="truncate">{label}</span>
          {hasSelection && !saving && (
            <span
              role="button"
              tabIndex={0}
              aria-label={t("chat.workingDir.clear")}
              onClick={handleClear}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault()
                  handleClear(e as unknown as React.MouseEvent)
                }
              }}
              className="ml-0.5 rounded hover:bg-muted p-0.5"
            >
              <X className="h-3 w-3" />
            </span>
          )}
        </button>
      </IconTip>
      {!isTauriMode() && (
        <ServerDirectoryBrowser
          open={browserOpen}
          initialPath={workingDir ?? null}
          onOpenChange={setBrowserOpen}
          onSelect={(path) => {
            setBrowserOpen(false)
            onChange(path)
          }}
        />
      )}
    </>
  )
}
