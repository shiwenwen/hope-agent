import { useTranslation } from "react-i18next"
import { useDangerousModeStatus } from "@/hooks/useDangerousModeStatus"
import { ShieldAlert } from "lucide-react"

/**
 * Persistent warning banner rendered while Dangerous Mode is active.
 *
 * Two sources (CLI flag OR config flag) drive the banner text:
 *   - config only        → guide the user to the Settings → Security toggle
 *   - CLI only           → instruct them to relaunch without the flag
 *   - both active        → acknowledge both and nudge toward the toggle
 *
 * Collapses to `null` when inactive so it occupies zero space.
 */
export default function DangerousModeBanner() {
  const { t } = useTranslation()
  const status = useDangerousModeStatus()

  if (!status.active) return null

  let message: string
  if (status.cliFlag && status.configFlag) {
    message = t(
      "dangerousMode.bannerBoth",
      "危险模式已启用（CLI + 配置）—— 所有工具调用跳过审批",
    )
  } else if (status.cliFlag) {
    message = t(
      "dangerousMode.bannerCli",
      "危险模式已启用（CLI 启动）—— 重启应用不带 --dangerously-skip-all-approvals 即可关闭",
    )
  } else {
    message = t(
      "dangerousMode.bannerConfig",
      "危险模式已启用 —— 所有工具调用跳过审批。前往 设置 → 安全 关闭",
    )
  }

  return (
    <div className="shrink-0 bg-destructive text-destructive-foreground px-4 py-1.5 flex items-center gap-2 text-xs font-medium">
      <ShieldAlert className="h-3.5 w-3.5 shrink-0" />
      <span className="truncate">{message}</span>
    </div>
  )
}
