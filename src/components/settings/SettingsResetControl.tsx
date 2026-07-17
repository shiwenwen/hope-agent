import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Loader2, RotateCcw } from "lucide-react"
import { toast } from "sonner"

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Button } from "@/components/ui/button"
import {
  normalizeChatDisplayMode,
  resetChatDisplayModePreference,
  writeChatDisplayModePreference,
} from "@/components/chat/chatDisplayModePreference"
import {
  emitCompletedTurnCollapsePreference,
  normalizeCompletedTurnCollapsePreference,
} from "@/components/chat/completedTurnCollapsePreference"
import {
  emitAutoSendPendingPreference,
  normalizeAutoSendPendingPreference,
} from "@/components/chat/autoSendPendingPreference"
import { invalidateThinkingExpandCache } from "@/components/chat/thinkingCache"
import { isTauriMode } from "@/lib/transport"
import { getTransport, switchToEmbedded } from "@/lib/transport-provider"
import { TauriTransport } from "@/lib/transport-tauri"
import type { SettingsSection } from "./types"
import {
  RESET_SCOPE_BY_SECTION,
  type SettingsResetLevel,
  type SettingsResetScope,
  type SettingsResetSection,
} from "./settingsReset"

interface SettingsResetResult {
  scope: SettingsResetScope
  section?: SettingsResetSection
  changed: boolean
  reindexStarted: boolean
  warningCodes: string[]
}

interface ResettableUserPreferences {
  autoSendPending?: unknown
  autoCollapseCompletedTurns?: unknown
  chatDisplayMode?: unknown
}

async function syncFrontendPreferences(
  scope: SettingsResetScope,
  section?: SettingsResetSection,
): Promise<void> {
  const transport = getTransport()

  const resetsGeneralAppearance = scope === "general" && (!section || section === "appearance")
  const resetsChatBasic = scope === "chat" && (!section || section === "basic")

  if (resetsGeneralAppearance || resetsChatBasic) {
    try {
      const config = await transport.call<ResettableUserPreferences>("get_user_config")
      const displayMode = normalizeChatDisplayMode(config.chatDisplayMode)
      if (displayMode) {
        writeChatDisplayModePreference(displayMode)
      } else {
        resetChatDisplayModePreference()
      }

      if (resetsChatBasic) {
        emitAutoSendPendingPreference(
          normalizeAutoSendPendingPreference(config.autoSendPending),
        )
        emitCompletedTurnCollapsePreference(
          normalizeCompletedTurnCollapsePreference(config.autoCollapseCompletedTurns),
        )
      }
    } catch {
      // The persisted reset already succeeded. Other config listeners still
      // provide best-effort refresh, so do not report the reset itself as failed.
    }
  }

  if (resetsChatBasic) {
    invalidateThinkingExpandCache()
  }

  if (resetsGeneralAppearance) {
    window.dispatchEvent(new Event("ui-effects-changed"))
    const { initLanguageFromConfig } = await import("@/i18n/i18n")
    await Promise.allSettled([
      initLanguageFromConfig(),
      transport.call<string>("get_sidebar_display_mode").then((mode) => {
        window.dispatchEvent(
          new CustomEvent("sidebar-display-mode-changed", { detail: { mode } }),
        )
      }),
    ])
  }
}

export default function SettingsResetControl({
  section,
  scope: explicitScope,
  resetSection,
  sectionLabel,
  level = "page",
  onReset,
  className,
  disabled = false,
}: {
  section?: SettingsSection
  scope?: SettingsResetScope
  resetSection?: SettingsResetSection
  sectionLabel: string
  level?: SettingsResetLevel
  onReset: (result: SettingsResetResult) => void | Promise<void>
  className?: string
  disabled?: boolean
}) {
  const { t } = useTranslation()
  const [dialogOpen, setDialogOpen] = useState(false)
  const [resetting, setResetting] = useState(false)
  const scope = explicitScope ?? (section ? RESET_SCOPE_BY_SECTION[section] : undefined)

  if (!scope) return null

  const reset = async () => {
    if (resetting) return
    setResetting(true)
    try {
      // A desktop client may currently be connected to a remote HTTP server.
      // Resetting the Server page must still update this app's embedded server
      // config before the active connection switches back to it.
      const resetTransport = scope === "server" && !resetSection && isTauriMode()
        ? new TauriTransport()
        : getTransport()
      const result = await resetTransport.call<SettingsResetResult>("reset_settings_section", {
        scope,
        ...(resetSection ? { section: resetSection } : {}),
      })
      if (scope === "server" && !resetSection) {
        switchToEmbedded({ dirtyConfirmed: true })
      }
      await syncFrontendPreferences(scope, resetSection)
      setDialogOpen(false)
      toast.success(
        result.changed
          ? t("settings.resetDefaultsSuccess", { section: sectionLabel })
          : t("settings.resetDefaultsAlreadyDefault", { section: sectionLabel }),
        {
          description: result.reindexStarted
            ? t("settings.resetDefaultsReindexStarted")
            : result.warningCodes.length > 0
              ? t("settings.resetDefaultsWarning")
              : undefined,
        },
      )
      try {
        await onReset(result)
      } catch {
        // Persistence already succeeded. Config listeners provide a second
        // refresh path, so a local re-read failure must not be reported as a
        // failed reset or roll back the committed defaults.
      }
    } catch (error) {
      toast.error(t("settings.resetDefaultsFailed", { section: sectionLabel }), {
        description: error instanceof Error ? error.message : String(error),
      })
    } finally {
      setResetting(false)
    }
  }

  return (
    <>
      <Button
        variant="outline"
        size="sm"
        className={
          className ?? (level === "page" ? "mb-1 h-7 gap-1.5" : "h-7 shrink-0 gap-1.5")
        }
        onClick={() => setDialogOpen(true)}
        disabled={disabled || resetting}
      >
        {resetting ? (
          <Loader2 className="h-3.5 w-3.5 animate-spin" />
        ) : (
          <RotateCcw className="h-3.5 w-3.5" />
        )}
        {level === "page"
          ? t("settings.resetDefaultsPageAction", "恢复本页默认值")
          : level === "tab"
            ? t("settings.resetDefaultsTabAction", "恢复当前页签")
            : t("settings.resetDefaultsRegionAction", "恢复此区域")}
      </Button>

      <AlertDialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t("settings.resetDefaultsTitle", { section: sectionLabel })}
            </AlertDialogTitle>
            <AlertDialogDescription asChild>
              <div className="space-y-2">
                <p>{t("settings.resetDefaultsDescription", { section: sectionLabel })}</p>
                <p>{t("settings.resetDefaultsPreserve")}</p>
                {scope === "knowledge" && !resetSection && (
                  <p className="text-amber-600 dark:text-amber-400">
                    {t("settings.resetDefaultsKnowledgeWarning")}
                  </p>
                )}
              </div>
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={disabled || resetting}>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              disabled={disabled || resetting}
              onClick={(event) => {
                event.preventDefault()
                void reset()
              }}
            >
              {resetting && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
              {t("common.restoreDefaults")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}
