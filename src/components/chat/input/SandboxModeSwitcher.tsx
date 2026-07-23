import { useCallback, useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { FloatingMenu } from "@/components/ui/floating-menu"
import { IconTip } from "@/components/ui/tooltip"
import { useClickOutside } from "@/hooks/useClickOutside"
import { cn } from "@/lib/utils"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { DockerSetupHint } from "@/components/settings/DockerSetupHint"
import type { DockerStatus } from "@/components/settings/dockerSetup"
import { Box, Copy, Folder, Shield, ShieldCheck } from "lucide-react"
import type { SandboxMode } from "@/types/chat"

const SESSION_SANDBOX_MODE_ORDER: ReadonlyArray<SandboxMode> = [
  "off",
  "standard",
  "isolated",
  "workspace",
  "trusted",
]

interface ModeTheme {
  Icon: typeof Shield
  buttonTone: string
  iconTone: string
}

const MODE_THEME: Record<SandboxMode, ModeTheme> = {
  off: {
    Icon: Shield,
    buttonTone: "text-muted-foreground hover:text-foreground",
    iconTone: "",
  },
  standard: {
    Icon: Box,
    buttonTone: "text-sky-600 dark:text-sky-400",
    iconTone: "text-sky-600 dark:text-sky-400",
  },
  isolated: {
    Icon: Copy,
    buttonTone: "text-emerald-600 dark:text-emerald-400",
    iconTone: "text-emerald-600 dark:text-emerald-400",
  },
  workspace: {
    Icon: Folder,
    buttonTone: "text-indigo-600 dark:text-indigo-400",
    iconTone: "text-indigo-600 dark:text-indigo-400",
  },
  trusted: {
    Icon: ShieldCheck,
    buttonTone: "text-amber-600 dark:text-amber-400",
    iconTone: "text-amber-600 dark:text-amber-400",
  },
}

export interface SandboxModeOptionsProps {
  sandboxMode: SandboxMode
  onSandboxModeChange: (mode: SandboxMode) => void
  /** Start Docker availability checks only while the containing menu is visible. */
  active?: boolean
  /** Called when a selection can close its containing menu. */
  onSelectionComplete?: () => void
}

/**
 * Reusable sandbox choice list. The input permission menu embeds this body so
 * permission and sandbox controls share one popover without duplicating the
 * Docker availability check or setup hint.
 */
export function SandboxModeOptions({
  sandboxMode,
  onSandboxModeChange,
  active = true,
  onSelectionComplete,
}: SandboxModeOptionsProps) {
  const { t } = useTranslation()
  const [status, setStatus] = useState<DockerStatus | null>(null)
  const [checking, setChecking] = useState(false)

  const refreshStatus = useCallback(async () => {
    setChecking(true)
    try {
      const s = await getTransport().call<DockerStatus>("check_sandbox_available")
      setStatus(s)
    } catch (e) {
      logger.error("chat", "SandboxModeSwitcher", "Failed to check Docker status", e)
    } finally {
      setChecking(false)
    }
  }, [])

  useEffect(() => {
    if (!active || sandboxMode === "off") return
    void refreshStatus()
  }, [active, refreshStatus, sandboxMode])

  const dockerReady = status?.installed && status?.running

  return (
    <div className="flex flex-col gap-0.5">
      {SESSION_SANDBOX_MODE_ORDER.map((mode) => {
        const theme = MODE_THEME[mode]
        const Icon = theme.Icon
        return (
          <button
            key={mode}
            className={cn(
              "w-full text-left px-2.5 py-2 rounded-md transition-all duration-150 flex items-start gap-2",
              sandboxMode === mode
                ? "bg-secondary text-foreground font-medium"
                : "text-foreground/80 hover:bg-secondary/60 hover:text-foreground",
            )}
            onClick={() => {
              onSandboxModeChange(mode)
              if (mode === "off" || dockerReady) {
                onSelectionComplete?.()
              } else {
                void refreshStatus()
              }
            }}
          >
            <Icon className={cn("h-4 w-4 mt-0.5 shrink-0", theme.iconTone)} />
            <div className="flex flex-col">
              <span className="text-[13px]">
                {t(`chat.sandboxMode.${mode}.label`, { defaultValue: mode })}
              </span>
              <span className="text-[11px] text-muted-foreground font-normal">
                {t(`chat.sandboxMode.${mode}.desc`, {
                  defaultValue: sandboxModeDescription(mode),
                })}
              </span>
            </div>
          </button>
        )
      })}
      {sandboxMode !== "off" && (!status || !dockerReady) && (
        <DockerSetupHint
          status={status}
          checking={checking}
          onRefresh={refreshStatus}
          title={t("chat.sandboxMode.setupTitle", {
            defaultValue: "配置 Docker 后启用沙箱",
          })}
          className="mt-1"
        />
      )}
    </div>
  )
}

export default function SandboxModeSwitcher({
  sandboxMode,
  onSandboxModeChange,
  variant = "toolbar",
}: {
  sandboxMode: SandboxMode
  onSandboxModeChange: (mode: SandboxMode) => void
  /** "toolbar" (default) = compact button + floating popover in the composer
   *  toolbar; "menu" = full-width accordion row for the composer "+" overflow
   *  when space is tight (expands inline instead of floating). */
  variant?: "toolbar" | "menu"
}) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const menuRef = useRef<HTMLDivElement>(null)

  useClickOutside(
    menuRef,
    useCallback(() => setOpen(false), []),
  )

  const activeTheme = MODE_THEME[sandboxMode]
  const ActiveIcon = activeTheme.Icon
  const activeLabel = t(`chat.sandboxMode.${sandboxMode}.label`, {
    defaultValue: sandboxMode,
  })
  const isMenu = variant === "menu"

  const modeListBody = (
    <SandboxModeOptions
      active={open}
      sandboxMode={sandboxMode}
      onSandboxModeChange={onSandboxModeChange}
      onSelectionComplete={() => setOpen(false)}
    />
  )

  return (
    <div className={cn("relative", isMenu && "w-full")} ref={menuRef}>
      {isMenu ? (
        <button
          type="button"
          aria-label={activeLabel}
          onClick={() => setOpen(!open)}
          className={cn(
            "flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-[13px] outline-none transition-all duration-150 hover:bg-secondary/60 hover:text-foreground",
            activeTheme.buttonTone,
          )}
        >
          <ActiveIcon className="h-4 w-4 shrink-0" />
          <span className="truncate">
            {t("chat.sandboxMode.menuLabel", { defaultValue: "沙箱" })}
          </span>
          <span className="ml-auto truncate text-xs text-muted-foreground">{activeLabel}</span>
        </button>
      ) : (
        <IconTip label={activeLabel}>
          <button
            type="button"
            aria-label={activeLabel}
            onClick={() => setOpen(!open)}
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
          className="min-w-[280px] p-1.5"
          onEscapeKeyDown={() => setOpen(false)}
        >
          {modeListBody}
        </FloatingMenu>
      )}
    </div>
  )
}

function sandboxModeDescription(mode: SandboxMode): string {
  switch (mode) {
    case "off":
      return "在宿主机执行，审批逻辑不变"
    case "standard":
      return "在 Docker 沙箱执行，审批不放松"
    case "isolated":
      return "隔离副本试跑，编辑审批不放松"
    case "workspace":
      return "挂载当前工作区，减少编辑命令审批"
    case "trusted":
      return "沙箱内 exec 最大自治，严格风险仍审批"
  }
}
