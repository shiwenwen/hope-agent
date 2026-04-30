import { useState, useRef, useCallback } from "react"
import { useTranslation } from "react-i18next"
import { useClickOutside } from "@/hooks/useClickOutside"
import { cn } from "@/lib/utils"
import { Shield, ShieldCheck, ShieldAlert } from "lucide-react"
import type { SessionMode } from "@/types/chat"

interface PermissionModeSwitcherProps {
  permissionMode: SessionMode
  onPermissionModeChange: (mode: SessionMode) => void
}

const MODE_ICON: Record<SessionMode, typeof Shield> = {
  default: Shield,
  smart: ShieldCheck,
  yolo: ShieldAlert,
}

const MODE_TONE_BUTTON: Record<SessionMode, string> = {
  default: "text-muted-foreground hover:text-foreground",
  smart: "text-amber-600 dark:text-amber-400",
  yolo: "text-destructive",
}

const MODE_TONE_ICON: Record<SessionMode, string> = {
  default: "",
  smart: "text-amber-600 dark:text-amber-400",
  yolo: "text-destructive",
}

const MODE_ORDER: ReadonlyArray<SessionMode> = ["default", "smart", "yolo"]

export default function PermissionModeSwitcher({
  permissionMode,
  onPermissionModeChange,
}: PermissionModeSwitcherProps) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const menuRef = useRef<HTMLDivElement>(null)

  useClickOutside(menuRef, useCallback(() => setOpen(false), []))

  const ActiveIcon = MODE_ICON[permissionMode]

  return (
    <div className="relative" ref={menuRef}>
      <button
        onClick={() => setOpen(!open)}
        className={cn(
          "flex items-center gap-1 bg-transparent text-xs font-medium px-2 py-1 rounded-lg cursor-pointer transition-colors hover:bg-secondary shrink-0 whitespace-nowrap",
          MODE_TONE_BUTTON[permissionMode],
        )}
      >
        <ActiveIcon className="h-3.5 w-3.5 shrink-0" />
        <span>{t(`chat.permissionMode.${permissionMode}.label`)}</span>
      </button>

      {open && (
        <div className="absolute bottom-full left-0 mb-2 bg-popover/95 backdrop-blur-xl border border-border/60 rounded-xl shadow-[0_8px_30px_rgb(0,0,0,0.12)] z-50 min-w-[200px] p-1.5 animate-in fade-in-0 zoom-in-95 slide-in-from-bottom-1 duration-150">
          <div className="flex flex-col gap-0.5">
            {MODE_ORDER.map((mode) => {
              const Icon = MODE_ICON[mode]
              return (
                <button
                  key={mode}
                  className={cn(
                    "w-full text-left px-2.5 py-2 rounded-md transition-all duration-150 flex items-start gap-2",
                    permissionMode === mode
                      ? "bg-secondary text-foreground font-medium shadow-sm"
                      : "text-foreground/80 hover:bg-secondary/60 hover:text-foreground",
                  )}
                  onClick={() => {
                    onPermissionModeChange(mode)
                    setOpen(false)
                  }}
                >
                  <Icon
                    className={cn(
                      "h-3.5 w-3.5 mt-0.5 shrink-0",
                      MODE_TONE_ICON[mode],
                    )}
                  />
                  <div className="flex flex-col">
                    <span className="text-[13px]">
                      {t(`chat.permissionMode.${mode}.label`)}
                    </span>
                    <span className="text-[11px] text-muted-foreground font-normal">
                      {t(`chat.permissionMode.${mode}.desc`)}
                    </span>
                  </div>
                </button>
              )
            })}
          </div>
        </div>
      )}
    </div>
  )
}
