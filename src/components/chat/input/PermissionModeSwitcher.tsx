import { useState, useRef, useCallback } from "react"
import { useTranslation } from "react-i18next"
import { Switch } from "@/components/ui/switch"
import { AnimatedCollapse } from "@/components/ui/animated-presence"
import { FloatingMenu } from "@/components/ui/floating-menu"
import { IconTip } from "@/components/ui/tooltip"
import { useClickOutside } from "@/hooks/useClickOutside"
import { cn } from "@/lib/utils"
import { ChevronDown, Shield, ShieldCheck, ShieldAlert } from "lucide-react"
import type { SandboxMode, SessionMode } from "@/types/chat"
import { SESSION_PERMISSION_MODE_ORDER } from "./permissionModes"
import { SandboxModeOptions } from "./SandboxModeSwitcher"

export interface PermissionModeChangeOptions {
  applyToAgentDefault?: boolean
}

interface PermissionModeSwitcherProps {
  permissionMode: SessionMode
  onPermissionModeChange: (mode: SessionMode, options?: PermissionModeChangeOptions) => void
  /** When provided, sandbox selection is rendered in this same popover. */
  sandboxMode?: SandboxMode
  onSandboxModeChange?: (mode: SandboxMode) => void
  /** "toolbar" (default) = compact button + floating popover in the composer
   *  toolbar; "menu" = full-width accordion row for the composer "+" overflow. */
  variant?: "toolbar" | "menu"
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
  sandboxMode,
  onSandboxModeChange,
  variant = "toolbar",
}: PermissionModeSwitcherProps) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [sandboxExpanded, setSandboxExpanded] = useState(false)
  const [applyToAgentDefault, setApplyToAgentDefault] = useState(false)
  const menuRef = useRef<HTMLDivElement>(null)
  const closeMenu = useCallback(() => {
    setOpen(false)
    setSandboxExpanded(false)
  }, [])

  useClickOutside(menuRef, closeMenu)

  const activeTheme = MODE_THEME[permissionMode]
  const ActiveIcon = activeTheme.Icon
  const activeLabel = t(`chat.permissionMode.${permissionMode}.label`)
  const isMenu = variant === "menu"
  const sandboxControls =
    sandboxMode !== undefined && onSandboxModeChange !== undefined
      ? { sandboxMode, onSandboxModeChange }
      : null
  const sandboxLabel = sandboxControls
    ? t(`chat.sandboxMode.${sandboxControls.sandboxMode}.label`, {
        defaultValue: sandboxControls.sandboxMode,
      })
    : null

  const modeListBody = (
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
              closeMenu()
            }}
          >
            <Icon className={cn("h-4 w-4 mt-0.5 shrink-0", theme.iconTone)} />
            <div className="flex flex-col">
              <span className="text-[13px]">{t(`chat.permissionMode.${mode}.label`)}</span>
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
      {sandboxControls && (
        <>
          <div className="my-1 h-px bg-border/60" />
          <button
            type="button"
            aria-label={t("chat.sandboxMode.menuLabel", { defaultValue: "沙箱" })}
            aria-expanded={sandboxExpanded}
            onClick={() => setSandboxExpanded((expanded) => !expanded)}
            className="flex w-full items-center justify-between gap-3 rounded-md px-2.5 py-1.5 text-left text-[13px] font-medium text-foreground/80 transition-colors hover:bg-secondary/60 hover:text-foreground"
          >
            <span>{t("chat.sandboxMode.menuLabel", { defaultValue: "沙箱" })}</span>
            <span className="ml-auto truncate text-xs font-normal text-muted-foreground">
              {sandboxLabel}
            </span>
            <ChevronDown
              className={cn(
                "h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform duration-200",
                sandboxExpanded && "rotate-180",
              )}
            />
          </button>
          <AnimatedCollapse open={sandboxExpanded}>
            <div className="pt-0.5">
              <SandboxModeOptions
                active={open && sandboxExpanded}
                sandboxMode={sandboxControls.sandboxMode}
                onSandboxModeChange={sandboxControls.onSandboxModeChange}
                onSelectionComplete={closeMenu}
              />
            </div>
          </AnimatedCollapse>
        </>
      )}
    </div>
  )

  return (
    <div className={cn("relative", isMenu && "w-full")} ref={menuRef}>
      {isMenu ? (
        <button
          type="button"
          aria-label={`${activeLabel} (Shift+Tab)`}
          onClick={() => (open ? closeMenu() : setOpen(true))}
          className={cn(
            "flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-[13px] outline-none transition-all duration-150 hover:bg-secondary/60 hover:text-foreground",
            activeTheme.buttonTone,
          )}
        >
          <ActiveIcon className="h-4 w-4 shrink-0" />
          <span className="truncate">
            {t("chat.permissionMode.menuLabel", { defaultValue: "权限模式" })}
          </span>
          <span className="ml-auto truncate text-xs text-muted-foreground">{activeLabel}</span>
        </button>
      ) : (
        <IconTip label={`${activeLabel} (Shift+Tab)`}>
          <button
            type="button"
            aria-label={`${activeLabel} (Shift+Tab)`}
            onClick={() => (open ? closeMenu() : setOpen(true))}
            className={cn(
              "flex items-center gap-1 bg-transparent text-xs font-medium px-2 py-1 rounded-lg cursor-pointer transition-colors hover:bg-secondary shrink-0 whitespace-nowrap",
              activeTheme.buttonTone,
            )}
          >
            <ActiveIcon className="h-4 w-4 shrink-0" />
            <span>{activeLabel}</span>
          </button>
        </IconTip>
      )}

      {isMenu ? (
        open && (
          <div className="mt-1 rounded-lg border border-border/50 bg-background/40 p-1.5 animate-in fade-in-0 slide-in-from-top-1 duration-150">
            {modeListBody}
          </div>
        )
      ) : (
        <FloatingMenu
          open={open}
          className="min-w-[280px] max-h-[min(560px,calc(100vh-96px))] overflow-y-auto overscroll-contain p-1.5"
          onEscapeKeyDown={closeMenu}
        >
          {modeListBody}
        </FloatingMenu>
      )}
    </div>
  )
}
