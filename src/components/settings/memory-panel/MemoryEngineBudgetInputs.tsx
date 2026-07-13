import { useTranslation } from "react-i18next"
import { DeferredNumberInput } from "@/components/ui/deferred-number-input"
import { Label } from "@/components/ui/label"
import type { MemoryRuntimeConfig } from "./types"

interface Props {
  value: MemoryRuntimeConfig
  onChange: (next: MemoryRuntimeConfig) => void
  disabled?: boolean
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
        onValueCommit={onCommit}
      />
    </div>
  )
}

export default function MemoryEngineBudgetInputs({ value, onChange, disabled }: Props) {
  const { t } = useTranslation()

  const updateCore = (patch: Partial<MemoryRuntimeConfig["core"]>) =>
    onChange({ ...value, core: { ...value.core, ...patch } })
  const updateRecall = (patch: Partial<MemoryRuntimeConfig["recall"]>) =>
    onChange({ ...value, recall: { ...value.recall, ...patch } })
  const updateDeep = (patch: Partial<MemoryRuntimeConfig["deepRecall"]>) =>
    onChange({ ...value, deepRecall: { ...value.deepRecall, ...patch } })

  return (
    <div className="space-y-5">
      <div className="space-y-2">
        <h4 className="text-xs font-medium text-muted-foreground">
          {t("settings.memoryV2.core.title")}
        </h4>
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 xl:grid-cols-4">
          <NumberField label={t("settings.memoryBudget.engine.totalTokens")} value={value.core.totalTokens} min={128} max={4096} disabled={disabled} onCommit={(totalTokens) => updateCore({ totalTokens })} />
          <NumberField label={t("settings.memoryBudget.engine.hardMaxTokens")} value={value.core.hardMaxTokens} min={256} max={4096} disabled={disabled} onCommit={(hardMaxTokens) => updateCore({ hardMaxTokens })} />
          <NumberField label={t("settings.memoryBudget.engine.globalTokens")} value={value.core.globalTokens} min={32} max={4096} disabled={disabled} onCommit={(globalTokens) => updateCore({ globalTokens })} />
          <NumberField label={t("settings.memoryBudget.engine.agentTokens")} value={value.core.agentTokens} min={32} max={4096} disabled={disabled} onCommit={(agentTokens) => updateCore({ agentTokens })} />
          <NumberField label={t("settings.memoryBudget.engine.projectTokens")} value={value.core.projectTokens} min={32} max={4096} disabled={disabled} onCommit={(projectTokens) => updateCore({ projectTokens })} />
          <NumberField label={t("settings.memoryBudget.engine.protocolTokens")} value={value.core.protocolTokens} min={32} max={4096} disabled={disabled} onCommit={(protocolTokens) => updateCore({ protocolTokens })} />
          <NumberField label={t("settings.memoryBudget.engine.topicReadTokens")} value={value.core.topicReadMaxTokens} min={64} max={4096} disabled={disabled} onCommit={(topicReadMaxTokens) => updateCore({ topicReadMaxTokens })} />
        </div>
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
