import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Loader2, Save } from "lucide-react"
import { Button } from "@/components/ui/button"
import { RadioPills } from "@/components/ui/radio-pills"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"

type UnattendedAction = "deny" | "proceed"

/**
 * Unattended-approval policy (Epic D). Decides what happens when a tool needs
 * approval but no human can answer (cron / headless-no-client / ACP-no-
 * capability / subagent-no-surface). `deny` fail-closes; `proceed` auto-runs —
 * a security loosening, hence a HIGH-risk setting.
 */
export default function UnattendedApprovalSection() {
  const { t } = useTranslation()
  const [action, setAction] = useState<UnattendedAction>("deny")
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")

  useEffect(() => {
    let cancelled = false
    getTransport()
      .call<UnattendedAction>("get_unattended_approval_action")
      .then((a) => {
        if (!cancelled) setAction(a)
      })
      .catch((e) => logger.error("settings", "unattendedApproval", "load failed", e))
      .finally(() => {
        if (!cancelled) setLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [])

  const save = async () => {
    setSaving(true)
    try {
      await getTransport().call("set_unattended_approval_action", { action })
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "unattendedApproval", "save failed", e)
      setSaveStatus("failed")
      setTimeout(() => setSaveStatus("idle"), 2000)
      toast.error(t("common.saveFailed"))
    } finally {
      setSaving(false)
    }
  }

  if (loading) return null

  return (
    <section className="rounded-lg border border-border/50 bg-card/40 p-4">
      <header className="mb-3">
        <h3 className="text-sm font-medium text-foreground">
          {t("settings.approvalPanel.unattendedTitle")}
        </h3>
        <p className="text-xs text-muted-foreground mt-0.5">
          {t("settings.approvalPanel.unattendedDesc")}
        </p>
      </header>

      <div className="max-w-xs">
        <RadioPills
          value={action}
          onChange={setAction}
          cols="grid-cols-2"
          options={(["deny", "proceed"] as const).map((a) => ({
            value: a,
            label: t(`settings.approvalPanel.timeoutActions.${a}`),
          }))}
        />
      </div>

      <div className="mt-3 flex justify-end">
        <Button
          size="sm"
          disabled={saving}
          onClick={save}
          className={`h-7 text-xs ${
            saveStatus === "saved" ? "bg-emerald-600 hover:bg-emerald-600/90" : ""
          } ${saveStatus === "failed" ? "bg-destructive hover:bg-destructive/90" : ""}`}
        >
          {saving ? (
            <Loader2 className="h-3.5 w-3.5 mr-1 animate-spin" />
          ) : (
            <Save className="h-3 w-3 mr-1" />
          )}
          {saving
            ? t("common.saving")
            : saveStatus === "saved"
              ? t("common.saved")
              : saveStatus === "failed"
                ? t("common.saveFailed")
                : t("common.save")}
        </Button>
      </div>
    </section>
  )
}
