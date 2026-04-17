import { useTranslation } from "react-i18next"
import { Slider } from "@/components/ui/slider"
import { isTauriMode } from "@/lib/transport"
import {
  useWindowOpacity,
  WINDOW_OPACITY_MIN,
  WINDOW_OPACITY_MAX,
} from "@/hooks/useWindowOpacity"

export default function WindowOpacitySection() {
  const { t } = useTranslation()
  const { opacity, setOpacity, loaded } = useWindowOpacity()

  if (!isTauriMode()) return null

  const percent = Math.round(opacity * 100)
  const minPercent = Math.round(WINDOW_OPACITY_MIN * 100)
  const maxPercent = Math.round(WINDOW_OPACITY_MAX * 100)

  return (
    <div>
      <h3 className="text-sm font-semibold text-foreground mb-1">
        {t("settings.windowOpacity")}
      </h3>
      <p className="text-xs text-muted-foreground mb-3">
        {t("settings.windowOpacityDesc")}
      </p>
      {loaded && (
        <div className="px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors">
          <div className="flex items-center justify-between mb-3">
            <span className="text-sm font-medium">
              {t("settings.windowOpacityValue")}
            </span>
            <span className="text-sm tabular-nums text-muted-foreground">
              {percent}%
            </span>
          </div>
          <Slider
            min={minPercent}
            max={maxPercent}
            step={1}
            value={[percent]}
            onValueChange={(vals) => {
              const next = (vals[0] ?? percent) / 100
              setOpacity(next)
            }}
          />
          <div className="flex justify-between text-[10px] text-muted-foreground mt-2 tabular-nums">
            <span>{minPercent}%</span>
            <span>{maxPercent}%</span>
          </div>
        </div>
      )}
    </div>
  )
}
