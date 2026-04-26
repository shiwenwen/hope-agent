import { useTranslation } from "react-i18next"
import { Switch } from "@/components/ui/switch"
import { Input } from "@/components/ui/input"
import { Button } from "@/components/ui/button"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import type { useMemoryData } from "./useMemoryData"

type MemoryData = ReturnType<typeof useMemoryData>

interface ExtractConfigProps {
  data: MemoryData
  isAgentMode: boolean
}

export default function ExtractConfig({ data, isAgentMode }: ExtractConfigProps) {
  const { t } = useTranslation()

  const {
    extractConfigLoaded,
    availableProviders,
    effectiveAutoExtract,
    effectiveProviderId,
    effectiveModelId,
    effectiveTokenThreshold,
    effectiveTimeThresholdSecs,
    effectiveMessageThreshold,
    effectiveIdleTimeoutSecs,
    agentHasOverride,
    handleToggleAutoExtract,
    handleUpdateExtractModel,
    handleUpdateTokenThreshold,
    handleUpdateTimeThresholdMins,
    handleUpdateMessageThreshold,
    handleUpdateIdleTimeoutMins,
    resetAgentExtract,
  } = data

  if (!extractConfigLoaded) return null

  return (
    <div className="rounded-lg bg-secondary/30 mb-4 shrink-0">
      <div className="flex items-center justify-between px-3 py-2">
        <div className="flex-1 min-w-0">
          <div className="text-sm font-medium flex items-center gap-1.5">
            {t("settings.memoryAutoExtract")}
            {isAgentMode && (
              <span className="text-[10px] font-normal text-muted-foreground/70">
                {agentHasOverride ? t("settings.memoryOverridden") : t("settings.memoryInherited")}
              </span>
            )}
          </div>
          <div className="text-xs text-muted-foreground">{t("settings.memoryAutoExtractDesc")}</div>
        </div>
        <Switch
          checked={effectiveAutoExtract}
          onCheckedChange={handleToggleAutoExtract}
        />
      </div>
      {effectiveAutoExtract && (
        <div className="px-3 pb-3 space-y-2 border-t border-border/30 pt-2">
          {/* Extraction model selector */}
          <div className="flex items-center gap-2">
            <label className="text-xs text-muted-foreground whitespace-nowrap min-w-[72px]">{t("settings.memoryExtractModel")}</label>
            <Select
              value={effectiveProviderId && effectiveModelId ? `${effectiveProviderId}::${effectiveModelId}` : "__chat__"}
              onValueChange={handleUpdateExtractModel}
            >
              <SelectTrigger className="h-7 text-xs flex-1">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="__chat__">{t("settings.memoryUseChatModel")}</SelectItem>
                {availableProviders.map((prov) =>
                  prov.models.map((m) => (
                    <SelectItem key={`${prov.id}::${m.id}`} value={`${prov.id}::${m.id}`}>
                      {prov.name} / {m.name}
                    </SelectItem>
                  ))
                )}
              </SelectContent>
            </Select>
          </div>
          {/* Token threshold */}
          <div className="flex items-center gap-2">
            <label className="text-xs text-muted-foreground whitespace-nowrap min-w-[72px]">{t("settings.memoryExtractTokenThreshold")}</label>
            <Input
              type="number"
              min={1000}
              max={50000}
              step={1000}
              value={effectiveTokenThreshold}
              onChange={(e) => handleUpdateTokenThreshold(parseInt(e.target.value) || 8000)}
              className="h-7 text-xs w-24"
            />
          </div>
          {/* Time threshold (displayed as minutes) */}
          <div className="flex items-center gap-2">
            <label className="text-xs text-muted-foreground whitespace-nowrap min-w-[72px]">{t("settings.memoryExtractTimeThreshold")}</label>
            <Input
              type="number"
              min={1}
              max={60}
              value={Math.round(effectiveTimeThresholdSecs / 60)}
              onChange={(e) => handleUpdateTimeThresholdMins(parseInt(e.target.value) || 5)}
              className="h-7 text-xs w-24"
            />
          </div>
          {/* Message threshold */}
          <div className="flex items-center gap-2">
            <label className="text-xs text-muted-foreground whitespace-nowrap min-w-[72px]">{t("settings.memoryExtractMessageThreshold")}</label>
            <Input
              type="number"
              min={2}
              max={50}
              value={effectiveMessageThreshold}
              onChange={(e) => handleUpdateMessageThreshold(parseInt(e.target.value) || 10)}
              className="h-7 text-xs w-24"
            />
          </div>
          {/* Idle timeout (displayed as minutes, 0 = disabled) */}
          <div className="flex items-center gap-2">
            <label className="text-xs text-muted-foreground whitespace-nowrap min-w-[72px]">{t("settings.memoryExtractIdleTimeout")}</label>
            <Input
              type="number"
              min={0}
              max={120}
              value={Math.round(effectiveIdleTimeoutSecs / 60)}
              onChange={(e) => handleUpdateIdleTimeoutMins(parseInt(e.target.value) || 0)}
              className="h-7 text-xs w-24"
            />
          </div>
          {/* Memory Flush (pre-compaction extraction) */}
          <div className="flex items-center justify-between pt-1">
            <div className="flex-1 min-w-0">
              <div className="text-xs text-muted-foreground">{t("settings.memoryFlushDesc")}</div>
            </div>
            <Switch
              checked={data.effectiveFlushBeforeCompact}
              onCheckedChange={data.handleToggleFlushBeforeCompact}
            />
          </div>
          {/* Reset to global (agent mode only) */}
          {isAgentMode && agentHasOverride && (
            <Button
              variant="ghost"
              size="sm"
              onClick={resetAgentExtract}
              className="h-auto -ml-2 px-2 py-1 text-[11px] font-normal text-muted-foreground underline underline-offset-2 hover:bg-transparent hover:text-foreground"
            >
              {t("settings.memoryResetToGlobal")}
            </Button>
          )}
        </div>
      )}
    </div>
  )
}
