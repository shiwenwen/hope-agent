import { useState, useCallback } from "react"
import { useTranslation } from "react-i18next"
import { invoke } from "@tauri-apps/api/core"
import { logger } from "@/lib/logger"
import { Button } from "@/components/ui/button"
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
import {
  MessageSquare,
  Clock,
  Brain,
  Settings2,
  Trash2,
  Loader2,
  AlertTriangle,
  Sparkles,
} from "lucide-react"

type ClearTarget = "sessions" | "cron" | "memory" | "config" | "all"

const CLEAR_ACTIONS: {
  target: ClearTarget
  icon: React.ReactNode
  labelKey: string
  descKey: string
  command: string
  destructive?: boolean
}[] = [
  {
    target: "sessions",
    icon: <MessageSquare className="h-4 w-4" />,
    labelKey: "settings.devClearSessions",
    descKey: "settings.devClearSessionsDesc",
    command: "dev_clear_sessions",
  },
  {
    target: "cron",
    icon: <Clock className="h-4 w-4" />,
    labelKey: "settings.devClearCron",
    descKey: "settings.devClearCronDesc",
    command: "dev_clear_cron",
  },
  {
    target: "memory",
    icon: <Brain className="h-4 w-4" />,
    labelKey: "settings.devClearMemory",
    descKey: "settings.devClearMemoryDesc",
    command: "dev_clear_memory",
  },
  {
    target: "config",
    icon: <Settings2 className="h-4 w-4" />,
    labelKey: "settings.devResetConfig",
    descKey: "settings.devResetConfigDesc",
    command: "dev_reset_config",
  },
  {
    target: "all",
    icon: <Trash2 className="h-4 w-4" />,
    labelKey: "settings.devClearAll",
    descKey: "settings.devClearAllDesc",
    command: "dev_clear_all",
    destructive: true,
  },
]

export default function DeveloperPanel() {
  const { t } = useTranslation()
  const [confirmTarget, setConfirmTarget] = useState<ClearTarget | null>(null)
  const [loading, setLoading] = useState<ClearTarget | null>(null)

  const handleClear = useCallback(async (target: ClearTarget) => {
    const action = CLEAR_ACTIONS.find((a) => a.target === target)
    if (!action) return

    setConfirmTarget(null)
    setLoading(target)

    try {
      await invoke(action.command)
      // Restart app to reinitialize databases
      await invoke("request_app_restart")
    } catch (e) {
      logger.error("settings", "DeveloperPanel::clearData", `Failed to clear ${target}`, e)
      setLoading(null)
    }
  }, [])

  const confirmAction = confirmTarget ? CLEAR_ACTIONS.find((a) => a.target === confirmTarget) : null

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="max-w-2xl mx-auto space-y-6">
        {/* Warning banner */}
        <div className="flex items-start gap-3 rounded-lg border border-destructive/30 bg-destructive/5 p-4">
          <AlertTriangle className="h-5 w-5 text-destructive shrink-0 mt-0.5" />
          <div className="space-y-1">
            <p className="text-sm font-medium text-destructive">{t("settings.devWarningTitle")}</p>
            <p className="text-sm text-muted-foreground">{t("settings.devWarningDesc")}</p>
          </div>
        </div>

        {/* Clear actions */}
        <div className="space-y-3">
          {CLEAR_ACTIONS.map((action) => (
            <div
              key={action.target}
              className="flex items-center justify-between rounded-lg border border-border p-4"
            >
              <div className="flex items-center gap-3 min-w-0">
                <span className={action.destructive ? "text-destructive" : "text-muted-foreground"}>
                  {action.icon}
                </span>
                <div className="min-w-0">
                  <p className="text-sm font-medium">{t(action.labelKey)}</p>
                  <p className="text-xs text-muted-foreground truncate">{t(action.descKey)}</p>
                </div>
              </div>
              <Button
                variant={action.destructive ? "destructive" : "outline"}
                size="sm"
                disabled={loading !== null}
                onClick={() => setConfirmTarget(action.target)}
              >
                {loading === action.target ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  t("settings.devClearButton")
                )}
              </Button>
            </div>
          ))}
        </div>

        {/* Visual testing */}
        <div>
          <h3 className="text-sm font-semibold text-foreground mb-3 flex items-center gap-2">
            <Sparkles className="h-4 w-4" />
            {t("settings.devVisualTest", "UI 动效测试")}
          </h3>
          <div className="grid grid-cols-2 sm:grid-cols-4 gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() => window.dispatchEvent(new CustomEvent("simulate-weather", { detail: { weatherCode: 0 } }))}
            >
              晴天 (Clear)
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={() => window.dispatchEvent(new CustomEvent("simulate-weather", { detail: { weatherCode: 3 } }))}
            >
              多云 (Cloudy)
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={() => window.dispatchEvent(new CustomEvent("simulate-weather", { detail: { weatherCode: 45 } }))}
            >
              雾 (Fog)
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={() => window.dispatchEvent(new CustomEvent("simulate-weather", { detail: { weatherCode: 61 } }))}
            >
              下雨 (Rain)
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={() => window.dispatchEvent(new CustomEvent("simulate-weather", { detail: { weatherCode: 71 } }))}
            >
              下雪 (Snow)
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={() => window.dispatchEvent(new CustomEvent("simulate-weather", { detail: { weatherCode: 95 } }))}
            >
              雷暴 (Thunder)
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={() => window.dispatchEvent(new CustomEvent("simulate-weather", { detail: { weatherCode: 61, windSpeed: 50 } }))}
            >
              大风+雨 (Wind)
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={() => window.dispatchEvent(new CustomEvent("simulate-weather", { detail: { weatherCode: 71, windSpeed: 45 } }))}
            >
              大风+雪 (Wind)
            </Button>
          </div>
          <p className="text-xs text-muted-foreground mt-2">
            点击以上按钮可以临时替换全局动效参数（不影响实际缓存，重新加载/重启后恢复默认）。<br/>
            提示：繁星与流星特效只在深色 (Dark) 主题下出现。
          </p>
        </div>
      </div>

      {/* Confirmation dialog */}
      <AlertDialog
        open={confirmTarget !== null}
        onOpenChange={(open) => !open && setConfirmTarget(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{confirmAction ? t(confirmAction.labelKey) : ""}</AlertDialogTitle>
            <AlertDialogDescription>{t("settings.devConfirmDesc")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={() => confirmTarget && handleClear(confirmTarget)}
            >
              {t("common.confirm")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
