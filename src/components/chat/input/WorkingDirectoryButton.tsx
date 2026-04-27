import { FolderCheck, FolderPlus, Loader2, X } from "lucide-react"
import { useTranslation } from "react-i18next"
import { IconTip } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import { isTauriMode } from "@/lib/transport"
import { basename } from "@/lib/path"
import ServerDirectoryBrowser from "./ServerDirectoryBrowser"
import { useDirectoryPicker } from "./useDirectoryPicker"

interface WorkingDirectoryButtonProps {
  sessionId: string | null
  workingDir: string | null | undefined
  /**
   * `workingDir` came from the parent project (session itself has no
   * override). Hides the clear affordance because clearing a value the
   * session never owned would be a silent no-op; the path is still shown
   * so `@`-mention and label stay coherent.
   */
  inherited?: boolean
  saving?: boolean
  disabled?: boolean
  variant?: "toolbar" | "menu"
  onPicked?: () => void
  /**
   * Fired with the canonical path (or `null` to clear). Parent is
   * responsible for persisting to the backend.
   */
  onChange: (workingDir: string | null) => void
}

export default function WorkingDirectoryButton({
  sessionId,
  workingDir,
  inherited = false,
  saving = false,
  disabled = false,
  variant = "toolbar",
  onPicked,
  onChange,
}: WorkingDirectoryButtonProps) {
  const { t } = useTranslation()
  const hasSelection = typeof workingDir === "string" && workingDir.length > 0
  const showClear = hasSelection && !inherited

  const { pick, browserOpen, setBrowserOpen, handleBrowserSelect } = useDirectoryPicker({
    onPicked: (path) => {
      onChange(path)
      onPicked?.()
    },
    errorTitle: t("chat.workingDir.invalid"),
    loggerSource: "WorkingDirectoryButton::pickLocalDirectory",
  })

  const handlePick = () => {
    if (disabled || saving) return
    void pick()
  }

  const handleClear = (e: React.MouseEvent) => {
    e.stopPropagation()
    if (disabled || saving) return
    onChange(null)
  }

  const tooltipLabel = hasSelection
    ? inherited
      ? `${t("chat.workingDir.titleBarInherited")}: ${workingDir}`
      : `${t("chat.workingDir.current")}: ${workingDir}`
    : sessionId
      ? t("chat.workingDir.select")
      : t("chat.workingDir.selectPreset")

  const label = hasSelection ? basename(workingDir!) : t("chat.workingDir.select")

  if (variant === "menu") {
    return (
      <div className="w-full">
        <button
          type="button"
          disabled={saving || disabled}
          onClick={handlePick}
          className={cn(
            "flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-[13px] outline-none transition-all duration-150 hover:bg-secondary/60 hover:text-foreground focus-visible:bg-secondary/60 focus-visible:text-foreground disabled:pointer-events-none disabled:opacity-50",
            saving && "disabled:opacity-70",
            hasSelection ? "text-primary" : "text-foreground",
          )}
        >
          {saving ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin shrink-0 text-muted-foreground" />
          ) : hasSelection ? (
            <FolderCheck className="h-3.5 w-3.5 shrink-0 text-primary" />
          ) : (
            <FolderPlus className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
          )}
          <span className="truncate">{hasSelection ? label : t("chat.addWorkingDirectory")}</span>
        </button>
        {!isTauriMode() && (
          <ServerDirectoryBrowser
            open={browserOpen}
            initialPath={workingDir ?? null}
            onOpenChange={setBrowserOpen}
            onSelect={handleBrowserSelect}
          />
        )}
      </div>
    )
  }

  return (
    <div className="flex items-center shrink-0">
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
            showClear && !saving && "rounded-r-none",
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
        </button>
      </IconTip>
      {showClear && !saving && (
        <IconTip label={t("chat.workingDir.clear")}>
          <button
            type="button"
            disabled={disabled}
            onClick={handleClear}
            aria-label={t("chat.workingDir.clear")}
            className={cn(
              "flex items-center bg-transparent px-1 py-1 rounded-r-lg cursor-pointer transition-colors hover:bg-secondary shrink-0 text-primary hover:text-primary disabled:cursor-not-allowed disabled:opacity-50",
            )}
          >
            <X className="h-3 w-3" />
          </button>
        </IconTip>
      )}
      {!isTauriMode() && (
        <ServerDirectoryBrowser
          open={browserOpen}
          initialPath={workingDir ?? null}
          onOpenChange={setBrowserOpen}
          onSelect={handleBrowserSelect}
        />
      )}
    </div>
  )
}
