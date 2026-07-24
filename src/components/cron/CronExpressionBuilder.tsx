import { useTranslation } from "react-i18next"
import { Input } from "@/components/ui/input"
import { RadioPills } from "@/components/ui/radio-pills"
import { TogglePills } from "@/components/ui/toggle-pills"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Code2 } from "lucide-react"
import type { CronFrequency } from "./CronJobForm.types"
import { WEEKDAY_KEYS } from "./cronHelpers"

interface CronExpressionBuilderProps {
  cronFreq: CronFrequency
  setCronFreq: (f: CronFrequency) => void
  cronHour: string
  setCronHour: (h: string) => void
  cronMinute: string
  setCronMinute: (m: string) => void
  cronWeekdays: boolean[]
  toggleWeekday: (idx: number) => void
  cronMonthDay: string
  setCronMonthDay: (d: string) => void
  cronRawExpr: string
  setCronRawExpr: (expr: string) => void
  cronExpression: string
}

export default function CronExpressionBuilder({
  cronFreq, setCronFreq,
  cronHour, setCronHour,
  cronMinute, setCronMinute,
  cronWeekdays, toggleWeekday,
  cronMonthDay, setCronMonthDay,
  cronRawExpr, setCronRawExpr,
  cronExpression,
}: CronExpressionBuilderProps) {
  const { t } = useTranslation()

  const hourOptions = Array.from({ length: 24 }, (_, i) => String(i).padStart(2, "0"))
  const minuteOptions = Array.from({ length: 12 }, (_, i) => String(i * 5).padStart(2, "0"))

  return (
    <div className="space-y-3">
      {/* Frequency pills */}
      <div>
        <label className="text-xs font-medium text-muted-foreground mb-1.5 block">
          {t("cron.frequency")}
        </label>
        <RadioPills<CronFrequency>
          value={cronFreq}
          onChange={setCronFreq}
          variant="strong"
          layout="wrap"
          itemClassName="px-3 py-1"
          ariaLabel={t("cron.frequency")}
          options={(["hourly", "daily", "weekly", "monthly", "custom"] as CronFrequency[]).map(
            (f) => ({ value: f, label: t(`cron.freq_${f}`) }),
          )}
        />
      </div>

      {/* Hourly: at minute */}
      {cronFreq === "hourly" && (
        <div className="flex items-center gap-2 text-xs">
          <span className="text-muted-foreground">{t("cron.atMinute")}</span>
          <Select value={cronMinute} onValueChange={setCronMinute}>
            <SelectTrigger className="w-20 h-8 text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {minuteOptions.map((m) => (
                <SelectItem key={m} value={m}>
                  {m}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <span className="text-muted-foreground">{t("cron.minuteOfHour")}</span>
        </div>
      )}

      {/* Daily: time picker */}
      {cronFreq === "daily" && (
        <div className="flex items-center gap-2 text-xs">
          <span className="text-muted-foreground">{t("cron.everyDayAt")}</span>
          <Select value={cronHour} onValueChange={setCronHour}>
            <SelectTrigger className="w-20 h-8 text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {hourOptions.map((h) => (
                <SelectItem key={h} value={h}>
                  {h}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <span>:</span>
          <Select value={cronMinute} onValueChange={setCronMinute}>
            <SelectTrigger className="w-20 h-8 text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {minuteOptions.map((m) => (
                <SelectItem key={m} value={m}>
                  {m}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      )}

      {/* Weekly: weekday toggles + time */}
      {cronFreq === "weekly" && (
        <div className="space-y-2">
          <TogglePills
            values={new Set(cronWeekdays.flatMap((enabled, index) => (enabled ? [index] : [])))}
            onToggle={toggleWeekday}
            ariaLabel={t("cron.frequency")}
            className="flex-nowrap gap-1"
            itemClassName="min-w-0 flex-1 px-1 py-1.5 font-medium"
            options={WEEKDAY_KEYS.map((key, index) => ({
              value: index,
              label: t(`cron.${key}`),
            }))}
          />
          <div className="flex items-center gap-2 text-xs">
            <span className="text-muted-foreground">{t("cron.atTime")}</span>
            <Select value={cronHour} onValueChange={setCronHour}>
              <SelectTrigger className="w-20 h-8 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {hourOptions.map((h) => (
                  <SelectItem key={h} value={h}>
                    {h}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <span>:</span>
            <Select value={cronMinute} onValueChange={setCronMinute}>
              <SelectTrigger className="w-20 h-8 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {minuteOptions.map((m) => (
                  <SelectItem key={m} value={m}>
                    {m}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </div>
      )}

      {/* Monthly: day of month + time */}
      {cronFreq === "monthly" && (
        <div className="space-y-2">
          <div className="flex items-center gap-2 text-xs">
            <span className="text-muted-foreground">{t("cron.everyMonthOn")}</span>
            <Select value={cronMonthDay} onValueChange={setCronMonthDay}>
              <SelectTrigger className="w-20 h-8 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {Array.from({ length: 31 }, (_, i) => String(i + 1)).map((d) => (
                  <SelectItem key={d} value={d}>
                    {d}
                    {t("cron.daySuffix")}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="flex items-center gap-2 text-xs">
            <span className="text-muted-foreground">{t("cron.atTime")}</span>
            <Select value={cronHour} onValueChange={setCronHour}>
              <SelectTrigger className="w-20 h-8 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {hourOptions.map((h) => (
                  <SelectItem key={h} value={h}>
                    {h}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <span>:</span>
            <Select value={cronMinute} onValueChange={setCronMinute}>
              <SelectTrigger className="w-20 h-8 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {minuteOptions.map((m) => (
                  <SelectItem key={m} value={m}>
                    {m}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </div>
      )}

      {/* Custom: raw cron expression */}
      {cronFreq === "custom" && (
        <div>
          <Input
            value={cronRawExpr}
            onChange={(e) => setCronRawExpr(e.target.value)}
            placeholder="0 0 9 * * *"
            className="font-mono text-sm"
          />
          <p className="text-[10px] text-muted-foreground mt-1">{t("cron.cronHelp")}</p>
        </div>
      )}

      {/* Generated expression preview (non-custom modes) */}
      {cronFreq !== "custom" && (
        <div className="flex items-center gap-2 text-[10px] text-muted-foreground bg-secondary/40 rounded-md px-2.5 py-1.5">
          <Code2 className="h-3 w-3 shrink-0" />
          <span className="font-mono">{cronExpression}</span>
          <button
            type="button"
            className="ml-auto text-primary hover:underline shrink-0"
            onClick={() => {
              setCronRawExpr(cronExpression)
              setCronFreq("custom")
            }}
          >
            {t("cron.editExpression")}
          </button>
        </div>
      )}
    </div>
  )
}
