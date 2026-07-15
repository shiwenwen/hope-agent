import { useState } from "react"
import { useTranslation } from "react-i18next"
import { AlertTriangle, ChevronRight } from "lucide-react"
import { Button } from "@/components/ui/button"
import { DeferredNumberInput } from "@/components/ui/deferred-number-input"
import { Label } from "@/components/ui/label"
import { cn } from "@/lib/utils"
import type { CoreMemoryBudgetStatus, MemoryRuntimeConfig } from "./types"

interface Props {
  value: MemoryRuntimeConfig
  onChange: (next: MemoryRuntimeConfig) => void
  coreBudgetStatus?: CoreMemoryBudgetStatus | null
  disabled?: boolean
}

const CORE_PRESETS = {
  concise: {
    totalTokens: 1000,
    globalTokens: 200,
    agentTokens: 275,
    projectTokens: 375,
    protocolTokens: 150,
  },
  balanced: {
    totalTokens: 1600,
    globalTokens: 350,
    agentTokens: 450,
    projectTokens: 650,
    protocolTokens: 150,
  },
  rich: {
    totalTokens: 2400,
    globalTokens: 500,
    agentTokens: 700,
    projectTokens: 1050,
    protocolTokens: 150,
  },
} as const

type CorePresetName = keyof typeof CORE_PRESETS

function matchesCorePreset(
  core: MemoryRuntimeConfig["core"],
  preset: (typeof CORE_PRESETS)[CorePresetName],
): boolean {
  return (
    core.totalTokens === preset.totalTokens &&
    core.globalTokens === preset.globalTokens &&
    core.agentTokens === preset.agentTokens &&
    core.projectTokens === preset.projectTokens &&
    core.protocolTokens === preset.protocolTokens
  )
}

interface NumberFieldProps {
  label: string
  value: number
  min: number
  max: number
  disabled?: boolean
  onCommit: (value: number) => void
}

function NumberField({ label, value, min, max, disabled, onCommit }: NumberFieldProps) {
  return (
    <div className="space-y-1">
      <Label className="text-[11px]">{label}</Label>
      <DeferredNumberInput
        min={min}
        max={max}
        disabled={disabled}
        value={value}
        aria-label={label}
        onValueCommit={onCommit}
      />
    </div>
  )
}

export default function MemoryEngineBudgetInputs({
  value,
  onChange,
  coreBudgetStatus,
  disabled,
}: Props) {
  const { t } = useTranslation()
  const [allocationExpanded, setAllocationExpanded] = useState(false)

  const updateCore = (patch: Partial<MemoryRuntimeConfig["core"]>) =>
    onChange({ ...value, core: { ...value.core, ...patch } })
  const updateRecall = (patch: Partial<MemoryRuntimeConfig["recall"]>) =>
    onChange({ ...value, recall: { ...value.recall, ...patch } })
  const updateDeep = (patch: Partial<MemoryRuntimeConfig["deepRecall"]>) =>
    onChange({ ...value, deepRecall: { ...value.deepRecall, ...patch } })

  const activePreset = (Object.entries(CORE_PRESETS) as Array<[
    CorePresetName,
    (typeof CORE_PRESETS)[CorePresetName],
  ]>).find(([, preset]) => matchesCorePreset(value.core, preset))?.[0]

  const applyPreset = (name: CorePresetName) => {
    const preset = CORE_PRESETS[name]
    updateCore({
      ...preset,
      // Compatibility-only field: keep old readers coherent without making
      // users manage a second visible ceiling.
      hardMaxTokens: Math.max(value.core.hardMaxTokens, preset.totalTokens),
    })
  }

  const configuredTokens = value.core.totalTokens
  const modelLimit = coreBudgetStatus?.modelSafetyLimitTokens ?? null
  const emergencyLimit = coreBudgetStatus?.emergencyLimitTokens ?? 16384
  const effectiveTokens = Math.min(configuredTokens, modelLimit ?? emergencyLimit, emergencyLimit)
  const isModelLimited = effectiveTokens < configuredTokens
  const isAboveRecommended = configuredTokens > 2400

  return (
    <div className="space-y-5">
      <div className="space-y-2">
        <h4 className="text-xs font-medium text-muted-foreground">
          {t("settings.memoryV2.core.title")}
        </h4>
        <p className="text-[11px] leading-4 text-muted-foreground">
          {t("settings.memoryBudget.coreSimpleDesc")}
        </p>
        <div className="flex flex-wrap gap-1.5">
          {(Object.keys(CORE_PRESETS) as CorePresetName[]).map((name) => (
            <Button
              key={name}
              type="button"
              size="sm"
              variant={activePreset === name ? "secondary" : "outline"}
              className="h-7 px-2 text-[11px]"
              disabled={disabled}
              onClick={() => applyPreset(name)}
            >
              {t(`settings.memoryBudget.presets.${name}`)} · {CORE_PRESETS[name].totalTokens}
            </Button>
          ))}
          {!activePreset && (
            <span className="inline-flex h-7 items-center rounded-md bg-muted px-2 text-[11px] text-muted-foreground">
              {t("common.custom")}
            </span>
          )}
        </div>
        <div className="grid gap-3 sm:grid-cols-[minmax(0,220px)_minmax(0,1fr)] sm:items-end">
          <NumberField
            label={t("settings.memoryBudget.engine.totalTokens")}
            value={configuredTokens}
            min={128}
            max={16384}
            disabled={disabled}
            onCommit={(totalTokens) => updateCore({
              totalTokens,
              hardMaxTokens: Math.max(value.core.hardMaxTokens, totalTokens),
            })}
          />
          <div className="rounded-md border border-border/60 bg-background/50 px-3 py-2 text-[11px] text-muted-foreground">
            <div>
              {t("settings.memoryBudget.effective", {
                configured: configuredTokens,
                effective: effectiveTokens,
              })}
            </div>
            {coreBudgetStatus?.contextWindowTokens != null && (
              <div className="mt-0.5">
                {t("settings.memoryBudget.contextWindow", {
                  count: coreBudgetStatus.contextWindowTokens,
                })}
              </div>
            )}
          </div>
        </div>
        {(isAboveRecommended || isModelLimited) && (
          <div className="flex gap-2 rounded-md border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-[11px] leading-4 text-amber-700 dark:text-amber-300">
            <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <div className="space-y-1">
              {isModelLimited && (
                <p>{t("settings.memoryBudget.modelLimited", {
                    configured: configuredTokens,
                    effective: effectiveTokens,
                  })}</p>
              )}
              {isAboveRecommended && (
                <p>{t("settings.memoryBudget.highBudgetWarning")}</p>
              )}
            </div>
          </div>
        )}
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="h-7 -ml-2 gap-1 px-2 text-[11px] text-muted-foreground"
          onClick={() => setAllocationExpanded((current) => !current)}
        >
          <ChevronRight className={cn("h-3.5 w-3.5 transition-transform", allocationExpanded && "rotate-90")} />
          {t("settings.memoryBudget.advancedAllocation")}
        </Button>
        {allocationExpanded && (
          <div className="grid grid-cols-2 gap-3 rounded-md border border-border/60 p-3 sm:grid-cols-3 xl:grid-cols-5">
            <NumberField label={t("settings.memoryBudget.engine.globalTokens")} value={value.core.globalTokens} min={32} max={16384} disabled={disabled} onCommit={(globalTokens) => updateCore({ globalTokens })} />
            <NumberField label={t("settings.memoryBudget.engine.agentTokens")} value={value.core.agentTokens} min={32} max={16384} disabled={disabled} onCommit={(agentTokens) => updateCore({ agentTokens })} />
            <NumberField label={t("settings.memoryBudget.engine.projectTokens")} value={value.core.projectTokens} min={32} max={16384} disabled={disabled} onCommit={(projectTokens) => updateCore({ projectTokens })} />
            <NumberField label={t("settings.memoryBudget.engine.protocolTokens")} value={value.core.protocolTokens} min={32} max={16384} disabled={disabled} onCommit={(protocolTokens) => updateCore({ protocolTokens })} />
            <NumberField label={t("settings.memoryBudget.engine.topicReadTokens")} value={value.core.topicReadMaxTokens} min={64} max={4096} disabled={disabled} onCommit={(topicReadMaxTokens) => updateCore({ topicReadMaxTokens })} />
          </div>
        )}
      </div>

      <div className="space-y-2">
        <h4 className="text-xs font-medium text-muted-foreground">
          {t("settings.memoryV2.recall.fast")}
        </h4>
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
          <NumberField label={t("settings.memoryBudget.engine.maxTokens")} value={value.recall.maxTokens} min={64} max={2400} disabled={disabled} onCommit={(maxTokens) => updateRecall({ maxTokens })} />
          <NumberField label={t("settings.memoryBudget.engine.maxSelected")} value={value.recall.maxSelected} min={1} max={20} disabled={disabled} onCommit={(maxSelected) => updateRecall({ maxSelected })} />
          <NumberField label={t("settings.memoryBudget.engine.candidateLimit")} value={value.recall.candidateLimit} min={1} max={100} disabled={disabled} onCommit={(candidateLimit) => updateRecall({ candidateLimit })} />
          <NumberField label={t("settings.memoryBudget.engine.timeoutMs")} value={value.recall.timeoutMs} min={20} max={2000} disabled={disabled} onCommit={(timeoutMs) => updateRecall({ timeoutMs })} />
        </div>
      </div>

      <div className="space-y-2">
        <h4 className="text-xs font-medium text-muted-foreground">
          {t("settings.memoryV2.recall.deep")}
        </h4>
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
          <NumberField label={t("settings.memoryBudget.engine.budgetTokens")} value={value.deepRecall.budgetTokens} min={64} max={2400} disabled={disabled} onCommit={(budgetTokens) => updateDeep({ budgetTokens })} />
          <NumberField label={t("settings.memoryBudget.engine.timeoutMs")} value={value.deepRecall.timeoutMs} min={500} max={15000} disabled={disabled} onCommit={(timeoutMs) => updateDeep({ timeoutMs })} />
          <NumberField label={t("settings.memoryBudget.engine.cacheTtlSecs")} value={value.deepRecall.cacheTtlSecs} min={10} max={3600} disabled={disabled} onCommit={(cacheTtlSecs) => updateDeep({ cacheTtlSecs })} />
          <NumberField label={t("settings.memoryBudget.engine.summaryChars")} value={value.deepRecall.maxChars} min={80} max={4000} disabled={disabled} onCommit={(maxChars) => updateDeep({ maxChars })} />
        </div>
      </div>
    </div>
  )
}
