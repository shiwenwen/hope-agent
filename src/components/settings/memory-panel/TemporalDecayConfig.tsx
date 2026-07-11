import { useCallback, useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { DeferredNumberInput } from "@/components/ui/deferred-number-input"
import { Switch } from "@/components/ui/switch"
import {
  memoryAdvancedConfigOperationErrorToast,
  type MemoryAdvancedConfigOperationErrorToast,
} from "./memoryAdvancedConfigFeedback"

interface TemporalDecayConfigData { enabled: boolean; halfLifeDays: number }

export default function TemporalDecayConfig() {
  const { t } = useTranslation()
  const [decayConfig, setDecayConfig] = useState<TemporalDecayConfigData>({ enabled: false, halfLifeDays: 30 })
  const [loadError, setLoadError] = useState<MemoryAdvancedConfigOperationErrorToast | null>(null)
  const saveSeqRef = useRef(0)

  const loadTemporalDecayConfig = useCallback(
    async (isCancelled?: () => boolean) => {
      try {
        const config = await getTransport().call<TemporalDecayConfigData>("get_temporal_decay_config")
        if (isCancelled?.()) return
        setDecayConfig(config)
        setLoadError(null)
      } catch (e) {
        logger.warn("settings", "TemporalDecayConfig::load", "Failed to load temporal decay", e)
        if (isCancelled?.()) return
        setLoadError(memoryAdvancedConfigOperationErrorToast("load", t, e))
      }
    },
    [t],
  )

  useEffect(() => {
    let cancelled = false
    queueMicrotask(() => {
      if (!cancelled) void loadTemporalDecayConfig(() => cancelled)
    })
    return () => {
      cancelled = true
    }
  }, [loadTemporalDecayConfig])

  const saveTemporalDecayConfig = useCallback(
    (updated: TemporalDecayConfigData, previous: TemporalDecayConfigData) => {
      const seq = saveSeqRef.current + 1
      saveSeqRef.current = seq
      setDecayConfig(updated)
      void getTransport()
        .call("save_temporal_decay_config", { config: updated })
        .catch((e) => {
          logger.error("settings", "TemporalDecayConfig::save", "Failed to save temporal decay", e)
          if (saveSeqRef.current !== seq) return
          setDecayConfig(previous)
          const failure = memoryAdvancedConfigOperationErrorToast("saveTemporal", t, e)
          toast.error(
            failure.title,
            failure.description ? { description: failure.description } : undefined,
          )
        })
    },
    [t],
  )

  return (
    <div className="space-y-2">
      {loadError && (
        <div className="rounded border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs">
          <div className="font-medium text-foreground">{loadError.title}</div>
          {loadError.description && (
            <div className="mt-1 break-all text-muted-foreground">{loadError.description}</div>
          )}
          <button
            type="button"
            className="mt-2 font-medium text-foreground underline underline-offset-2"
            onClick={() => void loadTemporalDecayConfig()}
          >
            {t("common.retry", "Retry")}
          </button>
        </div>
      )}
      <div className="flex items-center justify-between">
        <label className="text-xs font-medium">{t("settings.memoryTemporalDecay")}</label>
        <Switch
          checked={decayConfig.enabled}
          onCheckedChange={(v) => {
            const previous = decayConfig
            const updated = { ...decayConfig, enabled: v }
            saveTemporalDecayConfig(updated, previous)
          }}
        />
      </div>
      <p className="text-xs text-muted-foreground">{t("settings.memoryTemporalDecayDesc")}</p>
      {decayConfig.enabled && (
        <div className="flex items-center gap-2">
          <label className="text-xs text-muted-foreground whitespace-nowrap">{t("settings.memoryTemporalDecayHalfLife")}</label>
          <DeferredNumberInput
            min={1} max={365}
            value={decayConfig.halfLifeDays}
            integer={false}
            onValueCommit={(halfLifeDays) => {
              const previous = decayConfig
              const updated = { ...decayConfig, halfLifeDays }
              saveTemporalDecayConfig(updated, previous)
            }}
            className="h-7 text-xs w-20"
          />
          <span className="text-xs text-muted-foreground">{t("settings.memoryDays")}</span>
        </div>
      )}
    </div>
  )
}
