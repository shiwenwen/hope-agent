import { useTranslation } from "react-i18next"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"

const REASONING_EFFORTS = ["none", "minimal", "low", "medium", "high", "xhigh"] as const
const INHERIT_VALUE = "__inherit__"

interface ReasoningEffortSelectProps {
  value: string | null
  onChange: (value: string | null) => void
  inheritLabel?: string
  className?: string
}

export function ReasoningEffortSelect({
  value,
  onChange,
  inheritLabel,
  className,
}: ReasoningEffortSelectProps) {
  const { t } = useTranslation()
  const selectedValue = value || (inheritLabel ? INHERIT_VALUE : "medium")

  return (
    <Select
      value={selectedValue}
      onValueChange={(next) => onChange(next === INHERIT_VALUE ? null : next)}
    >
      <SelectTrigger className={className}>
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {inheritLabel && <SelectItem value={INHERIT_VALUE}>{inheritLabel}</SelectItem>}
        {REASONING_EFFORTS.map((effort) => (
          <SelectItem key={effort} value={effort}>
            {t(`effort.${effort}`)}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}
