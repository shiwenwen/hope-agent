import { useTranslation } from "react-i18next"

import { useEnhancedFocusIndicators } from "@/hooks/useEnhancedFocusIndicators"
import { Switch } from "@/components/ui/switch"

export default function FocusIndicatorSection() {
  const { t } = useTranslation()
  const { enabled, setEnabled } = useEnhancedFocusIndicators()

  return (
    <div className="flex items-center justify-between gap-4">
      <div className="min-w-0">
        <label htmlFor="enhanced-focus-indicators" className="text-sm font-medium text-foreground">
          {t("settings.enhancedFocusIndicators")}
        </label>
        <p
          id="enhanced-focus-indicators-description"
          className="mt-0.5 text-xs leading-relaxed text-muted-foreground"
        >
          {t("settings.enhancedFocusIndicatorsDesc")}
        </p>
      </div>
      <Switch
        id="enhanced-focus-indicators"
        checked={enabled}
        onCheckedChange={setEnabled}
        aria-describedby="enhanced-focus-indicators-description"
      />
    </div>
  )
}
