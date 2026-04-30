import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { ShieldAlert } from "lucide-react"
import { Switch } from "@/components/ui/switch"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"

interface GlobalYoloStatus {
  cliFlag: boolean
  configFlag: boolean
  active: boolean
}

export default function GlobalYoloSection() {
  const { t } = useTranslation()
  const [status, setStatus] = useState<GlobalYoloStatus | null>(null)
  const [busy, setBusy] = useState(false)

  const refresh = async () => {
    try {
      const s = await getTransport().call<GlobalYoloStatus>("get_global_yolo_status")
      setStatus(s)
    } catch (e) {
      logger.error("settings", "globalYolo", "get_global_yolo_status failed", e)
    }
  }

  useEffect(() => {
    void refresh()
  }, [])

  const toggle = async (enabled: boolean) => {
    setBusy(true)
    try {
      await getTransport().call("set_dangerous_skip_all_approvals", { enabled })
      await refresh()
      toast.success(
        enabled
          ? t("settings.approvalPanel.yoloEnabled")
          : t("settings.approvalPanel.yoloDisabled"),
      )
    } catch (e) {
      logger.error("settings", "globalYolo", "set_dangerous_skip_all_approvals failed", e)
      toast.error(t("settings.approvalPanel.saveFailed"))
    } finally {
      setBusy(false)
    }
  }

  if (!status) return null

  return (
    <section
      className={`rounded-lg border p-4 transition-colors ${
        status.active
          ? "border-destructive/40 bg-destructive/5"
          : "border-border/50 bg-card/40"
      }`}
    >
      <div className="flex items-start gap-3">
        <ShieldAlert
          className={`h-5 w-5 mt-0.5 shrink-0 ${
            status.active ? "text-destructive" : "text-muted-foreground"
          }`}
        />
        <div className="flex-1 min-w-0">
          <div className="flex items-start justify-between gap-3">
            <div>
              <h3 className="text-sm font-medium text-foreground">
                {t("settings.approvalPanel.yoloTitle")}
              </h3>
              <p className="text-xs text-muted-foreground mt-0.5">
                {t("settings.approvalPanel.yoloDesc")}
              </p>
            </div>
            <Switch
              checked={status.configFlag}
              onCheckedChange={toggle}
              disabled={busy}
            />
          </div>

          {status.cliFlag && (
            <div className="mt-2 rounded-md border border-amber-200/40 bg-amber-50/40 dark:bg-amber-950/10 px-2.5 py-1.5 text-[11px] text-amber-700 dark:text-amber-400">
              {t("settings.approvalPanel.yoloCliActive")}
            </div>
          )}
        </div>
      </div>
    </section>
  )
}
