import { useState, useEffect } from "react"
import { useTranslation } from "react-i18next"
import { getTransport } from "@/lib/transport-provider"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"

interface TemporalDecayConfigData { enabled: boolean; halfLifeDays: number }

export default function TemporalDecayConfig() {
  const { t } = useTranslation()
  const [decayConfig, setDecayConfig] = useState<TemporalDecayConfigData>({ enabled: false, halfLifeDays: 30 })

  useEffect(() => {
    getTransport().call<TemporalDecayConfigData>("get_temporal_decay_config").then(setDecayConfig).catch(() => {})
  }, [])

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <label className="text-xs font-medium">{t("settings.memoryTemporalDecay")}</label>
        <Switch
          checked={decayConfig.enabled}
          onCheckedChange={(v) => {
            const updated = { ...decayConfig, enabled: v }
            setDecayConfig(updated)
            getTransport().call("save_temporal_decay_config", { config: updated }).catch(() => {})
          }}
        />
      </div>
      <p className="text-xs text-muted-foreground">{t("settings.memoryTemporalDecayDesc")}</p>
      {decayConfig.enabled && (
        <div className="flex items-center gap-2">
          <label className="text-xs text-muted-foreground whitespace-nowrap">{t("settings.memoryTemporalDecayHalfLife")}</label>
          <Input
            type="number"
            min={1} max={365}
            value={decayConfig.halfLifeDays}
            onChange={(e) => {
              const val = parseFloat(e.target.value)
              if (!isNaN(val) && val > 0) {
                const updated = { ...decayConfig, halfLifeDays: val }
                setDecayConfig(updated)
                getTransport().call("save_temporal_decay_config", { config: updated }).catch(() => {})
              }
            }}
            className="h-7 text-xs w-20"
          />
          <span className="text-xs text-muted-foreground">{t("settings.memoryDays")}</span>
        </div>
      )}
    </div>
  )
}
