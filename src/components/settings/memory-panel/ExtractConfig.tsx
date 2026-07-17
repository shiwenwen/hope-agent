import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Switch } from "@/components/ui/switch"
import { DeferredNumberInput } from "@/components/ui/deferred-number-input"
import { Button } from "@/components/ui/button"
import { RadioPills } from "@/components/ui/radio-pills"
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import type { useMemoryData } from "./useMemoryData"
import SettingsResetControl from "../SettingsResetControl"

type MemoryData = ReturnType<typeof useMemoryData>

interface ExtractConfigProps {
  data: MemoryData
  isAgentMode: boolean
}

type MemoryLearningChoice = "automatic" | "review_first" | "manual_only" | "off"

export default function ExtractConfig({ data, isAgentMode }: ExtractConfigProps) {
  const { t } = useTranslation()
  const [offConfirmOpen, setOffConfirmOpen] = useState(false)

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
    effectiveMemoryLearningMode,
    applyMemoryLearningMode,
    handleUpdateExtractModel,
    handleUpdateTokenThreshold,
    handleUpdateTimeThresholdMins,
    handleUpdateMessageThreshold,
    handleUpdateIdleTimeoutMins,
    resetAgentExtract,
  } = data

  if (!extractConfigLoaded) return null

  const memoryLearningValue: MemoryLearningChoice | null =
    effectiveMemoryLearningMode === "automatic" ||
    effectiveMemoryLearningMode === "review_first" ||
    effectiveMemoryLearningMode === "manual_only" ||
    effectiveMemoryLearningMode === "off"
      ? effectiveMemoryLearningMode
      : null

  return (
    <>
      <div className="rounded-lg bg-secondary/30 mb-4 shrink-0">
        <div className="flex items-center justify-between gap-3 px-3 py-2">
          <div className="flex-1 min-w-0">
            <div className="text-sm font-medium flex items-center gap-1.5">
              {t("settings.memoryLearningMode")}
              {isAgentMode && (
                <span className="text-[10px] font-normal text-muted-foreground/70">
                  {agentHasOverride
                    ? t("settings.memoryOverridden")
                    : t("settings.memoryInherited")}
                </span>
              )}
            </div>
            <div className="text-xs text-muted-foreground">
              {t("settings.memoryLearningModeDesc")}
            </div>
          </div>
          {!isAgentMode && (
            <SettingsResetControl
              scope="memory"
              resetSection="extract"
              sectionLabel={t("settings.memoryLearningMode")}
              level="region"
              onReset={data.reloadExtractConfig}
            />
          )}
        </div>
        <div className="flex flex-wrap items-center gap-1.5 border-t border-border/30 px-3 py-2">
          <RadioPills<MemoryLearningChoice>
            value={memoryLearningValue}
            onChange={(mode) => {
              if (mode === "off") {
                setOffConfirmOpen(true)
                return
              }
              applyMemoryLearningMode(mode)
            }}
            variant="strong"
            layout="wrap"
            itemClassName="h-7 px-2 text-[11px]"
            ariaLabel={t("settings.memoryLearningMode")}
            options={[
              {
                value: "automatic",
                label: t("settings.memoryLearningModeAutomatic"),
              },
              ...(!isAgentMode
                ? [
                    {
                      value: "review_first" as const,
                      label: t("settings.memoryLearningModeReviewFirst"),
                    },
                  ]
                : []),
              {
                value: "manual_only",
                label: t("settings.memoryLearningModeManualOnly"),
              },
              ...(!isAgentMode
                ? [
                    {
                      value: "off" as const,
                      label: t("settings.memoryLearningModeOff"),
                    },
                  ]
                : []),
            ]}
          />
          {isAgentMode && effectiveMemoryLearningMode === "off" && (
            <span className="rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
              {t("settings.memoryLearningModeOff")}
            </span>
          )}
          {effectiveMemoryLearningMode === "custom" && (
            <span className="rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
              {t("common.custom")}
            </span>
          )}
        </div>
        {effectiveMemoryLearningMode === "off" && (
          <div className="border-t border-border/30 px-3 py-2 text-xs text-muted-foreground">
            {t("settings.memoryLearningModeOffDesc")}
          </div>
        )}
        {data.extractConfigError && (
          <div className="border-t border-border/30 px-3 py-2 text-xs text-destructive">
            <div className="font-medium">{data.extractConfigError.title}</div>
            {data.extractConfigError.description && (
              <div className="mt-0.5 text-destructive/80">
                {data.extractConfigError.description}
              </div>
            )}
          </div>
        )}
        {effectiveAutoExtract && (
          <div className="px-3 pb-3 space-y-2 border-t border-border/30 pt-2">
            {/* Extraction model selector */}
            <div className="flex items-center gap-2">
              <label className="text-xs text-muted-foreground whitespace-nowrap min-w-[72px]">
                {t("settings.memoryExtractModel")}
              </label>
              <Select
                value={
                  effectiveProviderId && effectiveModelId
                    ? `${effectiveProviderId}::${effectiveModelId}`
                    : "__chat__"
                }
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
                    )),
                  )}
                </SelectContent>
              </Select>
            </div>
            {/* Token threshold */}
            <div className="flex items-center gap-2">
              <label className="text-xs text-muted-foreground whitespace-nowrap min-w-[72px]">
                {t("settings.memoryExtractTokenThreshold")}
              </label>
              <DeferredNumberInput
                min={1000}
                max={50000}
                step={1000}
                value={effectiveTokenThreshold}
                onValueCommit={handleUpdateTokenThreshold}
                className="h-7 text-xs w-24"
              />
            </div>
            {/* Time threshold (displayed as minutes) */}
            <div className="flex items-center gap-2">
              <label className="text-xs text-muted-foreground whitespace-nowrap min-w-[72px]">
                {t("settings.memoryExtractTimeThreshold")}
              </label>
              <DeferredNumberInput
                min={1}
                max={60}
                value={Math.round(effectiveTimeThresholdSecs / 60)}
                onValueCommit={handleUpdateTimeThresholdMins}
                className="h-7 text-xs w-24"
              />
            </div>
            {/* Message threshold */}
            <div className="flex items-center gap-2">
              <label className="text-xs text-muted-foreground whitespace-nowrap min-w-[72px]">
                {t("settings.memoryExtractMessageThreshold")}
              </label>
              <DeferredNumberInput
                min={2}
                max={50}
                value={effectiveMessageThreshold}
                onValueCommit={handleUpdateMessageThreshold}
                className="h-7 text-xs w-24"
              />
            </div>
            {/* Idle timeout (displayed as minutes, 0 = disabled) */}
            <div className="flex items-center gap-2">
              <label className="text-xs text-muted-foreground whitespace-nowrap min-w-[72px]">
                {t("settings.memoryExtractIdleTimeout")}
              </label>
              <DeferredNumberInput
                min={0}
                max={120}
                value={Math.round(effectiveIdleTimeoutSecs / 60)}
                onValueCommit={handleUpdateIdleTimeoutMins}
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
                aria-label={t("settings.memoryFlushDesc")}
              />
            </div>
            {/* Structured claim dual-write — global-only */}
            {!isAgentMode && (
              <div className="flex items-center justify-between pt-1">
                <div className="flex-1 min-w-0">
                  <div className="text-xs font-medium flex items-center gap-1.5">
                    {t("settings.memoryExtractClaims")}
                    <span className="text-[9px] uppercase tracking-wide rounded bg-primary/15 text-primary px-1 py-0.5">
                      {t("settings.memoryStructuredBadge")}
                    </span>
                  </div>
                  <div className="text-xs text-muted-foreground">
                    {t("settings.memoryExtractClaimsDesc")}
                  </div>
                </div>
                <Switch
                  checked={data.effectiveExtractClaims}
                  onCheckedChange={data.handleToggleExtractClaims}
                  aria-label={t("settings.memoryExtractClaims")}
                />
              </div>
            )}
          </div>
        )}
        {isAgentMode && agentHasOverride && (
          <div className="border-t border-border/30 px-3 pb-2 pt-1">
            <Button
              variant="ghost"
              size="sm"
              onClick={resetAgentExtract}
              className="h-auto -ml-2 px-2 py-1 text-[11px] font-normal text-muted-foreground underline underline-offset-2 hover:bg-transparent hover:text-foreground"
            >
              {t("settings.memoryResetToGlobal")}
            </Button>
          </div>
        )}
      </div>
      <AlertDialog open={offConfirmOpen} onOpenChange={setOffConfirmOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("settings.memoryLearningModeOffConfirmTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("settings.memoryLearningModeOffConfirmDesc")}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={() => {
                applyMemoryLearningMode("off")
                setOffConfirmOpen(false)
              }}
            >
              {t("settings.memoryLearningModeOffConfirmAction")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}
