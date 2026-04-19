import { useCallback, useState } from "react"
import { useTranslation } from "react-i18next"
import { getTransport } from "@/lib/transport-provider"
import { useDangerousModeStatus } from "@/hooks/useDangerousModeStatus"
import { logger } from "@/lib/logger"
import { Switch } from "@/components/ui/switch"
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
import { ShieldAlert, Terminal } from "lucide-react"

const SKIPPED_CATEGORIES: Array<{ key: string; fallback: string }> = [
  { key: "settings.dangerousSkipsExec", fallback: "执行 shell 命令 (exec)" },
  { key: "settings.dangerousSkipsWrite", fallback: "写入 / 编辑文件 (write / edit / apply_patch)" },
  { key: "settings.dangerousSkipsBrowser", fallback: "浏览器工具" },
  { key: "settings.dangerousSkipsCanvas", fallback: "Canvas 画布工具" },
  { key: "settings.dangerousSkipsChannel", fallback: "所有 IM 渠道触发的工具调用" },
]

export default function DangerousModeSection() {
  const { t } = useTranslation()
  const status = useDangerousModeStatus()

  const [dialogOpen, setDialogOpen] = useState(false)
  const [ackChecked, setAckChecked] = useState(false)
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")

  const cliLocked = status.cliFlag

  const applyChange = useCallback(
    async (next: boolean) => {
      setSaving(true)
      try {
        await getTransport().call("set_dangerous_skip_all_approvals", { enabled: next })
        setSaveStatus("saved")
        setTimeout(() => setSaveStatus("idle"), 2000)
      } catch (e) {
        logger.error(
          "settings",
          "SecuritySection::save",
          "Failed to save dangerous mode",
          e,
        )
        setSaveStatus("failed")
        setTimeout(() => setSaveStatus("idle"), 2000)
      } finally {
        setSaving(false)
      }
    },
    [],
  )

  const handleToggle = useCallback(
    (nextChecked: boolean) => {
      if (cliLocked) return
      if (nextChecked) {
        setAckChecked(false)
        setDialogOpen(true)
      } else {
        void applyChange(false)
      }
    },
    [applyChange, cliLocked],
  )

  const handleConfirm = useCallback(() => {
    if (!ackChecked) return
    setDialogOpen(false)
    void applyChange(true)
  }, [ackChecked, applyChange])

  return (
    <div className="space-y-4">
      <p className="text-xs text-muted-foreground">
        {t(
          "settings.dangerousIntro",
          "一键跳过全部工具审批。极高风险，仅在完全信任的本地环境使用。",
        )}
      </p>

      <div className="rounded-lg border border-destructive/40 bg-destructive/5 p-4 space-y-3">
        <div className="flex items-start gap-3">
          <ShieldAlert className="h-5 w-5 text-destructive shrink-0 mt-0.5" />
          <div className="flex-1 min-w-0">
            <div className="flex items-center justify-between gap-3">
              <div className="space-y-0.5">
                <div className="text-sm font-medium">
                  {t("settings.dangerousSwitchLabel", "跳过全部工具审批")}
                </div>
                <div className="text-xs text-muted-foreground">
                  {t(
                    "settings.dangerousSwitchDesc",
                    "打开后所有工具调用无需审批。覆盖全局、会话级、渠道级所有审批设置。",
                  )}
                </div>
              </div>
              <Switch
                checked={status.configFlag || status.cliFlag}
                disabled={saving || cliLocked}
                onCheckedChange={handleToggle}
              />
            </div>

            {cliLocked && (
              <div className="mt-3 flex items-start gap-2 rounded-md bg-background/60 border border-border px-3 py-2 text-xs">
                <Terminal className="h-3.5 w-3.5 shrink-0 mt-0.5 text-muted-foreground" />
                <span className="text-muted-foreground">
                  {t(
                    "settings.dangerousCliLocked",
                    "当前由 CLI flag --dangerously-skip-all-approvals 启动，该开关在本次运行中只读。退出应用并不带此 flag 重新启动即可关闭。",
                  )}
                </span>
              </div>
            )}

            {saveStatus === "saved" && (
              <div className="mt-2 text-xs text-green-600 dark:text-green-400">
                {t("common.saved", "已保存")}
              </div>
            )}
            {saveStatus === "failed" && (
              <div className="mt-2 text-xs text-destructive">
                {t("common.saveFailed", "保存失败")}
              </div>
            )}
          </div>
        </div>
      </div>

      <AlertDialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle className="flex items-center gap-2 text-destructive">
              <ShieldAlert className="h-5 w-5" />
              {t("settings.dangerousConfirmTitle", "启用危险模式？")}
            </AlertDialogTitle>
            <AlertDialogDescription asChild>
              <div className="space-y-3 text-sm text-muted-foreground">
                <p>
                  {t(
                    "settings.dangerousConfirmBody",
                    "开启后以下操作将全部跳过审批，直接执行：",
                  )}
                </p>
                <ul className="list-disc pl-5 space-y-1">
                  {SKIPPED_CATEGORIES.map((item) => (
                    <li key={item.key}>{t(item.key, item.fallback)}</li>
                  ))}
                </ul>
                <p className="text-xs">
                  {t(
                    "settings.dangerousConfirmNote",
                    "Plan Mode 的工具类型限制仍然生效。你可以随时在本页关闭此开关。",
                  )}
                </p>
              </div>
            </AlertDialogDescription>
          </AlertDialogHeader>

          <label className="flex items-start gap-3 rounded-md border border-border px-3 py-2.5 cursor-pointer hover:bg-muted/40 transition-colors">
            <input
              type="checkbox"
              checked={ackChecked}
              onChange={(e) => setAckChecked(e.target.checked)}
              className="mt-0.5 h-4 w-4 rounded border-border accent-destructive"
            />
            <span className="text-sm">
              {t(
                "settings.dangerousConfirmAck",
                "我已了解风险，并同意跳过全部工具审批",
              )}
            </span>
          </label>

          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel", "取消")}</AlertDialogCancel>
            <AlertDialogAction asChild>
              <Button
                variant="destructive"
                disabled={!ackChecked}
                onClick={handleConfirm}
              >
                {t("settings.dangerousConfirmEnable", "启用危险模式")}
              </Button>
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
