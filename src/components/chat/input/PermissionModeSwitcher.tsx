import { useState, useRef, useCallback } from "react"
import { useTranslation } from "react-i18next"
import { Switch } from "@/components/ui/switch"
import { useClickOutside } from "@/hooks/useClickOutside"
import { cn } from "@/lib/utils"
import { Shield, ShieldCheck, ShieldAlert } from "lucide-react"
import type { SessionMode } from "@/types/chat"
import { SESSION_PERMISSION_MODE_ORDER } from "./permissionModes"

export interface PermissionModeChangeOptions {
  applyToAgentDefault?: boolean
}

interface PermissionModeSwitcherProps {
  permissionMode: SessionMode
  onPermissionModeChange: (mode: SessionMode, options?: PermissionModeChangeOptions) => void
}

interface ModeTheme {
  Icon: typeof Shield
  buttonTone: string
  iconTone: string
}

const MODE_THEME: Record<SessionMode, ModeTheme> = {
  default: {
    Icon: Shield,
    buttonTone: "text-muted-foreground hover:text-foreground",
    iconTone: "",
  },
  smart: {
    Icon: ShieldCheck,
    buttonTone: "text-amber-600 dark:text-amber-400",
    iconTone: "text-amber-600 dark:text-amber-400",
  },
  yolo: {
    Icon: ShieldAlert,
    buttonTone: "text-destructive",
    iconTone: "text-destructive",
  },
}

export default function PermissionModeSwitcher({
  permissionMode,
  onPermissionModeChange,
}: PermissionModeSwitcherProps) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [applyToAgentDefault, setApplyToAgentDefault] = useState(false)
  const menuRef = useRef<HTMLDivElement>(null)

  useClickOutside(menuRef, useCallback(() => setOpen(false), []))

  const activeTheme = MODE_THEME[permissionMode]
  const ActiveIcon = activeTheme.Icon
  const activeLabel = t(`chat.permissionMode.${permissionMode}.label`)

  return (
    <div className="relative" ref={menuRef}>
      <button
        type="button"
        aria-label={`${activeLabel} (Shift+Tab)`}
        title={`${activeLabel} (Shift+Tab)`}
        onClick={() => setOpen(!open)}
        className={cn(
          "flex items-center gap-1 bg-transparent text-xs font-medium px-2 py-1 rounded-lg cursor-pointer transition-colors hover:bg-secondary shrink-0 whitespace-nowrap",
          activeTheme.buttonTone,
        )}
      >
        <ActiveIcon className="h-4 w-4 shrink-0" />
        <span>{activeLabel}</span>
      </button>

      {open && (
        <div className="absolute bottom-full left-0 mb-2 bg-popover/95 backdrop-blur-xl border border-border/60 rounded-xl shadow-[0_8px_30px_rgb(0,0,0,0.12)] z-50 min-w-[240px] p-1.5 animate-in fade-in-0 zoom-in-95 slide-in-from-bottom-1 duration-150">
          <div className="flex flex-col gap-0.5">
            {SESSION_PERMISSION_MODE_ORDER.map((mode) => {
              const theme = MODE_THEME[mode]
              const Icon = theme.Icon
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
                    onPermissionModeChange(mode, {
                      applyToAgentDefault,
                    })
                    setOpen(false)
                  }}
                >
                  <Icon className={cn("h-4 w-4 mt-0.5 shrink-0", theme.iconTone)} />
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
            <div className="my-1 h-px bg-border/60" />
            <div className="flex items-center gap-3 px-2.5 py-2">
              <div className="min-w-0 flex-1">
                <div className="text-[13px] font-medium text-foreground">
                  {t("chat.permissionMode.applyToAgentDefault.label")}
                </div>
                <div className="text-[11px] leading-snug text-muted-foreground">
                  {t("chat.permissionMode.applyToAgentDefault.desc")}
                </div>
              </div>
              <Switch
                checked={applyToAgentDefault}
                onCheckedChange={setApplyToAgentDefault}
                aria-label={t("chat.permissionMode.applyToAgentDefault.label")}
              />
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
