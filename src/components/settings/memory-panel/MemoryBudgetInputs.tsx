import { useTranslation } from "react-i18next"
import { DeferredNumberInput } from "@/components/ui/deferred-number-input"
import { Label } from "@/components/ui/label"
import type {
  MemoryBudgetConfig,
  SqliteSectionBudgets,
} from "../types"

interface Props {
  value: MemoryBudgetConfig
  onChange: (next: MemoryBudgetConfig) => void
  disabled?: boolean
}

/// Shared numeric-input grid used by both the global MemoryPanel and the
/// per-agent MemoryTab override. Renders three top-level chars fields plus the
/// five per-section sub-budgets. `disabled` is used by the Agent tab when
/// "Use global default" is active.
export default function MemoryBudgetInputs({ value, onChange, disabled }: Props) {
  const { t } = useTranslation()

  const setField = (patch: Partial<MemoryBudgetConfig>) => onChange({ ...value, ...patch })
  const setSection = (patch: Partial<SqliteSectionBudgets>) =>
    onChange({ ...value, sqliteSections: { ...value.sqliteSections, ...patch } })

  return (
    <div className="space-y-4">
      <div className="grid grid-cols-1 gap-4 md:grid-cols-3">
        <div className="space-y-1.5">
          <Label className="text-xs">
            {t("settings.memoryBudget.totalChars")}
          </Label>
          <DeferredNumberInput
            min={0}
            disabled={disabled}
            value={value.totalChars}
            onValueCommit={(next) => setField({ totalChars: next })}
          />
          <p className="text-[11px] text-muted-foreground">
            {t("settings.memoryBudget.totalCharsDesc")}
          </p>
        </div>
        <div className="space-y-1.5">
          <Label className="text-xs">
            {t("settings.memoryBudget.coreMemoryFileChars")}
          </Label>
          <DeferredNumberInput
            min={0}
            disabled={disabled}
            value={value.coreMemoryFileChars}
            onValueCommit={(next) => setField({ coreMemoryFileChars: next })}
          />
          <p className="text-[11px] text-muted-foreground">
            {t("settings.memoryBudget.coreMemoryFileCharsDesc")}
          </p>
        </div>
        <div className="space-y-1.5">
          <Label className="text-xs">
            {t("settings.memoryBudget.sqliteEntryMaxChars")}
          </Label>
          <DeferredNumberInput
            min={0}
            disabled={disabled}
            value={value.sqliteEntryMaxChars}
            onValueCommit={(next) => setField({ sqliteEntryMaxChars: next })}
          />
          <p className="text-[11px] text-muted-foreground">
            {t("settings.memoryBudget.sqliteEntryMaxCharsDesc")}
          </p>
        </div>
      </div>

      <div>
        <h4 className="mb-2 text-xs font-medium text-muted-foreground">
          {t("settings.memoryBudget.sqliteSectionsTitle")}
        </h4>
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 xl:grid-cols-5">
          <div className="space-y-1">
            <Label className="text-[11px]">
              {t("settings.memoryBudget.sections.userProfile")}
            </Label>
            <DeferredNumberInput
              min={0}
              disabled={disabled}
              value={value.sqliteSections.userProfile}
              onValueCommit={(next) => setSection({ userProfile: next })}
            />
          </div>
          <div className="space-y-1">
            <Label className="text-[11px]">
              {t("settings.memoryBudget.sections.aboutUser")}
            </Label>
            <DeferredNumberInput
              min={0}
              disabled={disabled}
              value={value.sqliteSections.aboutUser}
              onValueCommit={(next) => setSection({ aboutUser: next })}
            />
          </div>
          <div className="space-y-1">
            <Label className="text-[11px]">
              {t("settings.memoryBudget.sections.preferences")}
            </Label>
            <DeferredNumberInput
              min={0}
              disabled={disabled}
              value={value.sqliteSections.preferences}
              onValueCommit={(next) => setSection({ preferences: next })}
            />
          </div>
          <div className="space-y-1">
            <Label className="text-[11px]">
              {t("settings.memoryBudget.sections.projectContext")}
            </Label>
            <DeferredNumberInput
              min={0}
              disabled={disabled}
              value={value.sqliteSections.projectContext}
              onValueCommit={(next) => setSection({ projectContext: next })}
            />
          </div>
          <div className="space-y-1">
            <Label className="text-[11px]">
              {t("settings.memoryBudget.sections.references")}
            </Label>
            <DeferredNumberInput
              min={0}
              disabled={disabled}
              value={value.sqliteSections.references}
              onValueCommit={(next) => setSection({ references: next })}
            />
          </div>
        </div>
      </div>
    </div>
  )
}
