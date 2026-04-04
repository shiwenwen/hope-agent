import { useState, useRef, useCallback } from "react"
import { useTranslation } from "react-i18next"
import { useClickOutside } from "@/hooks/useClickOutside"
import { IconTip } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import { Thermometer } from "lucide-react"
import { Slider } from "@/components/ui/slider"

interface TemperatureSliderProps {
  sessionTemperature?: number | null
  onSessionTemperatureChange?: (temp: number | null) => void
}

export default function TemperatureSlider({
  sessionTemperature,
  onSessionTemperatureChange,
}: TemperatureSliderProps) {
  const { t } = useTranslation()
  const [showTempMenu, setShowTempMenu] = useState(false)
  const tempMenuRef = useRef<HTMLDivElement>(null)

  useClickOutside(tempMenuRef, useCallback(() => setShowTempMenu(false), []))

  return (
    <div className="relative" ref={tempMenuRef}>
      <IconTip label={t("settings.temperature")}>
        <button
          onClick={() => setShowTempMenu(!showTempMenu)}
          className={cn(
            "flex items-center gap-1 bg-transparent text-xs font-medium px-2 py-1 rounded-lg cursor-pointer transition-colors hover:bg-secondary shrink-0 whitespace-nowrap",
            sessionTemperature != null
              ? "text-orange-500"
              : "text-muted-foreground hover:text-foreground",
          )}
        >
          <Thermometer className="h-3.5 w-3.5 shrink-0" />
          {sessionTemperature != null && (
            <span>{sessionTemperature.toFixed(1)}</span>
          )}
        </button>
      </IconTip>

      {showTempMenu && (
        <div className="absolute bottom-full left-0 mb-2 bg-popover/95 backdrop-blur-xl border border-border/60 rounded-xl shadow-[0_8px_30px_rgb(0,0,0,0.12)] z-50 w-[200px] p-3 animate-in fade-in-0 zoom-in-95 slide-in-from-bottom-1 duration-150">
          <div className="flex items-center justify-between mb-2">
            <span className="text-[11px] text-muted-foreground font-medium">
              {t("settings.temperature")}
            </span>
            <span className="text-xs font-mono text-foreground tabular-nums">
              {sessionTemperature != null ? sessionTemperature.toFixed(2) : t("settings.temperatureDefault")}
            </span>
          </div>
          <Slider
            min={0}
            max={200}
            step={1}
            value={[sessionTemperature != null ? Math.round(sessionTemperature * 100) : 100]}
            onValueChange={([v]) => {
              onSessionTemperatureChange?.(v / 100)
            }}
          />
          <div className="flex items-center justify-between mt-2">
            <span className="text-[10px] text-muted-foreground/60">{t("settings.temperaturePrecise")}</span>
            <button
              className="text-[10px] text-primary hover:text-primary/80 transition-colors"
              onClick={() => {
                onSessionTemperatureChange?.(null)
                setShowTempMenu(false)
              }}
            >
              {t("settings.temperatureReset")}
            </button>
          </div>
        </div>
      )}
    </div>
  )
}
