import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { Button } from "@/components/ui/button"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Check, Monitor } from "lucide-react"
import { type UserConfig, type TextInputProps, TIMEZONE_OPTIONS } from "./types"

interface PersonalInfoSectionProps {
  config: UserConfig
  update: (patch: Partial<UserConfig>) => void
  textInputProps: (field: keyof UserConfig) => TextInputProps
}

export default function PersonalInfoSection({
  config,
  update,
  textInputProps,
}: PersonalInfoSectionProps) {
  const { t } = useTranslation()

  return (
    <>
      {/* ── Birthday ── */}
      <div>
        <div className="text-xs font-medium text-muted-foreground mb-2 px-1">
          {t("settings.profileBirthday")}
        </div>
        <Input
          type="date"
          className="bg-secondary/40 rounded-lg"
          value={config.birthday ?? ""}
          onChange={(e) => {
            update({ birthday: e.target.value || null })
          }}
        />
        {config.birthday &&
          (() => {
            const bd = new Date(config.birthday + "T00:00:00")
            if (isNaN(bd.getTime())) return null
            const today = new Date()
            let age = today.getFullYear() - bd.getFullYear()
            const hadBirthdayThisYear =
              today.getMonth() > bd.getMonth() ||
              (today.getMonth() === bd.getMonth() && today.getDate() >= bd.getDate())
            if (!hadBirthdayThisYear) age -= 1
            const isBirthday =
              today.getMonth() === bd.getMonth() && today.getDate() === bd.getDate()
            return (
              <div className="mt-2 px-1 flex items-center gap-2">
                <span className="text-xs text-muted-foreground">
                  {t("settings.profileAgeDisplay", { age })}
                </span>
                {isBirthday && (
                  <span className="text-xs font-medium text-amber-500 animate-pulse">
                    🎂 {t("settings.profileBirthdaySurprise")}
                  </span>
                )}
              </div>
            )
          })()}
      </div>

      {/* ── Role ── */}
      <div>
        <div className="text-xs font-medium text-muted-foreground mb-2 px-1">
          {t("settings.profileRole")}
        </div>
        <Input
          className="bg-secondary/40 rounded-lg"
          {...textInputProps("role")}
          placeholder={t("settings.profileRolePlaceholder")}
        />
      </div>

      {/* ── AI Experience ── */}
      <div>
        <div className="text-xs font-medium text-muted-foreground mb-2 px-1">
          {t("settings.profileAiExperience")}
        </div>
        <div className="space-y-0.5">
          {(["expert", "intermediate", "beginner"] as const).map((level) => (
            <Button
              key={level}
              variant="ghost"
              className={cn(
                "h-auto w-full justify-start gap-3 rounded-lg px-3 py-2.5 text-sm",
                config.aiExperience === level
                  ? "bg-primary/10 text-primary font-medium hover:bg-primary/10 hover:text-primary"
                  : "bg-secondary/20 text-foreground hover:bg-secondary/60",
              )}
              onClick={() =>
                update({ aiExperience: config.aiExperience === level ? null : level })
              }
            >
              <span className="flex-1 text-left">
                {t(`settings.profileAiExp${level.charAt(0).toUpperCase() + level.slice(1)}`)}
              </span>
              {config.aiExperience === level && (
                <Check className="h-4 w-4 text-primary shrink-0" />
              )}
            </Button>
          ))}
        </div>
      </div>

      <div className="border-t border-border/50" />

      {/* ── Timezone ── */}
      <div>
        <div className="text-xs font-medium text-muted-foreground mb-2 px-1">
          {t("settings.profileTimezone")}
        </div>
        <div className="space-y-0.5">
          <Button
            variant="ghost"
            className={cn(
              "h-auto w-full justify-start gap-3 rounded-lg px-3 py-2.5 text-sm",
              !config.timezone
                ? "bg-primary/10 text-primary font-medium hover:bg-primary/10 hover:text-primary"
                : "bg-secondary/20 text-foreground hover:bg-secondary/60",
            )}
            onClick={() => update({ timezone: null })}
          >
            <Monitor className="h-4 w-4 shrink-0 opacity-60" />
            <span className="flex-1 text-left">{t("settings.profileTimezoneSystem")}</span>
            {!config.timezone && <Check className="h-4 w-4 text-primary shrink-0" />}
          </Button>
        </div>
        <Select
          value={config.timezone ?? ""}
          onValueChange={(v) => update({ timezone: v || null })}
        >
          <SelectTrigger className="mt-1 bg-secondary/20 text-sm hover:bg-secondary/60">
            <SelectValue placeholder={t("settings.profileTimezoneSystem")} />
          </SelectTrigger>
          <SelectContent>
            {TIMEZONE_OPTIONS.map((group) => (
              <SelectGroup key={group.groupKey}>
                <SelectLabel>{group.groupKey}</SelectLabel>
                {group.zones.map((tz) => (
                  <SelectItem key={tz.value} value={tz.value}>
                    {t(tz.labelKey)}
                  </SelectItem>
                ))}
              </SelectGroup>
            ))}
          </SelectContent>
        </Select>
      </div>

      <div className="border-t border-border/50" />

      {/* ── Custom Info ── */}
      <div>
        <div className="text-xs font-medium text-muted-foreground mb-2 px-1">
          {t("settings.profileCustomInfo")}
        </div>
        <Textarea
          className="bg-secondary/40 rounded-lg resize-none leading-relaxed"
          rows={5}
          {...textInputProps("customInfo")}
          placeholder={t("settings.profileCustomInfoPlaceholder")}
        />
      </div>
    </>
  )
}
