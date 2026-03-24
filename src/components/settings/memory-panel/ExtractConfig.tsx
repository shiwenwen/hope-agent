import { useTranslation } from "react-i18next"
import { Switch } from "@/components/ui/switch"
import { Input } from "@/components/ui/input"
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
    effectiveMinTurns,
    effectiveProviderId,
    effectiveModelId,
    agentHasOverride,
    handleToggleAutoExtract,
    handleUpdateExtractModel,
    handleUpdateExtractMinTurns,
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
          {/* Min turns */}
          <div className="flex items-center gap-2">
            <label className="text-xs text-muted-foreground whitespace-nowrap min-w-[72px]">{t("settings.memoryExtractMinTurns")}</label>
            <Input
              type="number"
              min={1}
              max={20}
              value={effectiveMinTurns}
              onChange={(e) => handleUpdateExtractMinTurns(parseInt(e.target.value) || 3)}
              className="h-7 text-xs w-20"
            />
          </div>
          {/* Reset to global (agent mode only) */}
          {isAgentMode && agentHasOverride && (
            <button
              onClick={resetAgentExtract}
              className="text-[11px] text-muted-foreground hover:text-foreground transition-colors underline underline-offset-2"
            >
              {t("settings.memoryResetToGlobal")}
            </button>
          )}
        </div>
      )}
    </div>
  )
}
