import { useTranslation } from "react-i18next"
import { ShieldCheck, Zap } from "lucide-react"

import { Switch } from "@/components/ui/switch"
import { Label } from "@/components/ui/label"

interface SafetyStepProps {
  approvalsEnabled: boolean
  onChange: (enabled: boolean) => void
}

/**
 * Step 5 — approvals explainer + single toggle.
 *
 * Dangerous Mode is deliberately not exposed here; the dedicated
 * Settings → Security panel requires an explicit double-confirm. See
 * AGENTS.md "HIGH risk category" rule.
 */
export function SafetyStep({ approvalsEnabled, onChange }: SafetyStepProps) {
  const { t } = useTranslation()
  return (
    <div className="px-6 py-6 space-y-5 max-w-xl mx-auto">
      <div className="text-center space-y-1">
        <h2 className="text-xl font-semibold">{t("onboarding.safety.title")}</h2>
        <p className="text-sm text-muted-foreground">{t("onboarding.safety.subtitle")}</p>
      </div>

      <div className="rounded-lg border border-border bg-muted/40 p-4 space-y-3 text-sm">
        <div className="flex items-start gap-3">
          <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary">
            <ShieldCheck className="h-5 w-5" />
          </div>
          <div>
            <div className="font-medium">{t("onboarding.safety.point1.title")}</div>
            <p className="text-muted-foreground">{t("onboarding.safety.point1.desc")}</p>
          </div>
        </div>
        <div className="flex items-start gap-3">
          <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary">
            <Zap className="h-5 w-5" />
          </div>
          <div>
            <div className="font-medium">{t("onboarding.safety.point2.title")}</div>
            <p className="text-muted-foreground">{t("onboarding.safety.point2.desc")}</p>
          </div>
        </div>
      </div>

      <div className="flex items-center justify-between rounded-md border border-border px-4 py-3">
        <div>
          <Label htmlFor="onb-approvals" className="text-sm font-medium">
            {t("onboarding.safety.toggle")}
          </Label>
          <p className="text-xs text-muted-foreground mt-0.5">
            {t("onboarding.safety.toggleHint")}
          </p>
        </div>
        <Switch
          id="onb-approvals"
          checked={approvalsEnabled}
          onCheckedChange={onChange}
        />
      </div>

      <div className="rounded-md border border-amber-500/30 bg-amber-500/5 px-4 py-3 text-xs text-amber-700 dark:text-amber-300">
        {t("onboarding.safety.dangerousHint")}
      </div>
    </div>
  )
}
