import { useState } from "react"
import { useTranslation } from "react-i18next"
import { RotateCcw } from "lucide-react"

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

import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"

/**
 * Settings entry-point for re-opening the first-run wizard.
 *
 * Calls `reset_onboarding` on the backend (which clears
 * `completed_version` but not providers or UserConfig) and then reloads
 * the window so App.tsx re-queries the state and routes to the wizard.
 */
export default function OnboardingResetSection() {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)

  async function confirm() {
    setOpen(false)
    try {
      await getTransport().call("reset_onboarding")
    } catch (e) {
      logger.error("settings", "rerun_onboarding", "reset_onboarding failed", e)
      return
    }
    // Full reload is the cheapest way to re-run the App.tsx boot flow,
    // which is where the wizard routing lives.
    window.location.reload()
  }

  return (
    <section className="space-y-2">
      <h3 className="text-sm font-medium">{t("onboarding.rerun.title")}</h3>
      <p className="text-xs text-muted-foreground max-w-prose">
        {t("onboarding.rerun.desc")}
      </p>
      <Button variant="outline" size="sm" onClick={() => setOpen(true)}>
        <RotateCcw className="h-3.5 w-3.5 mr-1" />
        {t("onboarding.rerun.button")}
      </Button>

      <AlertDialog open={open} onOpenChange={setOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("onboarding.rerun.title")}</AlertDialogTitle>
            <AlertDialogDescription>{t("onboarding.rerun.desc")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction onClick={confirm}>
              {t("onboarding.rerun.confirm")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  )
}
