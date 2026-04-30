import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Loader2, Save } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { RadioPills } from "@/components/ui/radio-pills"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"

type TimeoutAction = "deny" | "proceed"

export default function ApprovalTimeoutSection() {
  const { t } = useTranslation()
  const [seconds, setSeconds] = useState<number>(300)
  const [action, setAction] = useState<TimeoutAction>("deny")
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")

  useEffect(() => {
    let cancelled = false
    Promise.all([
      getTransport().call<number>("get_approval_timeout"),
      getTransport().call<TimeoutAction>("get_approval_timeout_action"),
    ])
      .then(([s, a]) => {
        if (cancelled) return
        setSeconds(s)
        setAction(a)
      })
      .catch((e) => logger.error("settings", "approvalTimeout", "load failed", e))
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
      await Promise.all([
        getTransport().call("set_approval_timeout", { seconds }),
        getTransport().call("set_approval_timeout_action", { action }),
      ])
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "approvalTimeout", "save failed", e)
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
          {t("settings.approvalPanel.timeoutTitle")}
        </h3>
        <p className="text-xs text-muted-foreground mt-0.5">
          {t("settings.approvalPanel.timeoutDesc")}
        </p>
      </header>

      <div className="space-y-3">
        <div>
          <label className="text-xs font-medium text-foreground/80">
            {t("settings.approvalPanel.timeoutSeconds")}
          </label>
          <div className="flex gap-2 items-center mt-1.5">
            <Input
              type="number"
              min={0}
              max={3600}
              value={seconds}
              onChange={(e) => setSeconds(Math.max(0, Number(e.target.value) || 0))}
              className="text-xs h-8 w-32"
            />
            <span className="text-[11px] text-muted-foreground">
              {t("settings.approvalPanel.timeoutHint")}
            </span>
          </div>
        </div>

        <div>
          <label className="text-xs font-medium text-foreground/80">
            {t("settings.approvalPanel.timeoutAction")}
          </label>
          <div className="mt-1.5 max-w-xs">
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
        </div>
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
