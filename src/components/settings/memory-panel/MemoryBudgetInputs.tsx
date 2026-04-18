import { useTranslation } from "react-i18next"
import { Input } from "@/components/ui/input"
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

  const parseNum = (s: string, fallback: number) => {
    const n = parseInt(s, 10)
    return Number.isFinite(n) && n >= 0 ? n : fallback
  }

  return (
    <div className="space-y-4">
      <div className="grid grid-cols-1 gap-4 md:grid-cols-3">
        <div className="space-y-1.5">
          <Label className="text-xs">
            {t("settings.memoryBudget.totalChars")}
          </Label>
          <Input
            type="number"
            min={0}
            disabled={disabled}
            value={value.totalChars}
            onChange={(e) => setField({ totalChars: parseNum(e.target.value, value.totalChars) })}
          />
          <p className="text-[11px] text-muted-foreground">
            {t("settings.memoryBudget.totalCharsDesc")}
          </p>
        </div>
        <div className="space-y-1.5">
          <Label className="text-xs">
            {t("settings.memoryBudget.coreMemoryFileChars")}
          </Label>
          <Input
            type="number"
            min={0}
            disabled={disabled}
            value={value.coreMemoryFileChars}
            onChange={(e) =>
              setField({ coreMemoryFileChars: parseNum(e.target.value, value.coreMemoryFileChars) })
            }
          />
          <p className="text-[11px] text-muted-foreground">
            {t("settings.memoryBudget.coreMemoryFileCharsDesc")}
          </p>
        </div>
        <div className="space-y-1.5">
          <Label className="text-xs">
            {t("settings.memoryBudget.sqliteEntryMaxChars")}
          </Label>
          <Input
            type="number"
            min={0}
            disabled={disabled}
            value={value.sqliteEntryMaxChars}
            onChange={(e) =>
              setField({ sqliteEntryMaxChars: parseNum(e.target.value, value.sqliteEntryMaxChars) })
            }
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
              {t("settings.memoryBudget.sections.aboutYou")}
            </Label>
            <Input
              type="number"
              min={0}
              disabled={disabled}
              value={value.sqliteSections.aboutYou}
              onChange={(e) =>
                setSection({ aboutYou: parseNum(e.target.value, value.sqliteSections.aboutYou) })
              }
            />
          </div>
          <div className="space-y-1">
            <Label className="text-[11px]">
              {t("settings.memoryBudget.sections.aboutUser")}
            </Label>
            <Input
              type="number"
              min={0}
              disabled={disabled}
              value={value.sqliteSections.aboutUser}
              onChange={(e) =>
                setSection({ aboutUser: parseNum(e.target.value, value.sqliteSections.aboutUser) })
              }
            />
          </div>
          <div className="space-y-1">
            <Label className="text-[11px]">
              {t("settings.memoryBudget.sections.preferences")}
            </Label>
            <Input
              type="number"
              min={0}
              disabled={disabled}
              value={value.sqliteSections.preferences}
              onChange={(e) =>
                setSection({
                  preferences: parseNum(e.target.value, value.sqliteSections.preferences),
                })
              }
            />
          </div>
          <div className="space-y-1">
            <Label className="text-[11px]">
              {t("settings.memoryBudget.sections.projectContext")}
            </Label>
            <Input
              type="number"
              min={0}
              disabled={disabled}
              value={value.sqliteSections.projectContext}
              onChange={(e) =>
                setSection({
                  projectContext: parseNum(e.target.value, value.sqliteSections.projectContext),
                })
              }
            />
          </div>
          <div className="space-y-1">
            <Label className="text-[11px]">
              {t("settings.memoryBudget.sections.references")}
            </Label>
            <Input
              type="number"
              min={0}
              disabled={disabled}
              value={value.sqliteSections.references}
              onChange={(e) =>
                setSection({
                  references: parseNum(e.target.value, value.sqliteSections.references),
                })
              }
            />
          </div>
        </div>
      </div>
    </div>
  )
}
