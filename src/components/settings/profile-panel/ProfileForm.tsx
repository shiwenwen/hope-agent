import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { Button } from "@/components/ui/button"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Check } from "lucide-react"
import {
  type UserConfig,
  type TextInputProps,
  GENDER_PRESETS,
  LANGUAGE_OPTIONS,
  PRESET_STYLES,
} from "./types"

interface ProfileFormProps {
  config: UserConfig
  customStyle: boolean
  customGender: boolean
  onCustomStyleChange: (v: boolean) => void
  onCustomGenderChange: (v: boolean) => void
  update: (patch: Partial<UserConfig>) => void
  textInputProps: (field: keyof UserConfig) => TextInputProps
}

export default function ProfileForm({
  config,
  customStyle,
  customGender,
  onCustomStyleChange,
  onCustomGenderChange,
  update,
  textInputProps,
}: ProfileFormProps) {
  const { t } = useTranslation()

  return (
    <>
      {/* ── Name ── */}
      <div>
        <div className="text-xs font-medium text-muted-foreground mb-2 px-1">
          {t("settings.profileName")}
        </div>
        <Input
          className="bg-secondary/40 rounded-lg"
          {...textInputProps("name")}
          placeholder={t("settings.profileNamePlaceholder")}
        />
      </div>

      {/* ── Gender ── */}
      <div>
        <div className="text-xs font-medium text-muted-foreground mb-2 px-1">
          {t("settings.profileGender")}
        </div>
        <div className="space-y-0.5">
          {GENDER_PRESETS.map((g) => (
            <Button
              key={g}
              variant="ghost"
              className={cn(
                "h-auto w-full justify-start gap-3 rounded-lg px-3 py-2.5 text-sm",
                !customGender && config.gender === g
                  ? "bg-primary/10 text-primary font-medium hover:bg-primary/10 hover:text-primary"
                  : "bg-secondary/20 text-foreground hover:bg-secondary/60",
              )}
              onClick={() => {
                onCustomGenderChange(false)
                update({ gender: config.gender === g ? null : g })
              }}
            >
              <span className="flex-1 text-left">
                {t(`settings.profileGender${g.charAt(0).toUpperCase() + g.slice(1)}`)}
              </span>
              {!customGender && config.gender === g && (
                <Check className="h-4 w-4 text-primary shrink-0" />
              )}
            </Button>
          ))}
          <Button
            variant="ghost"
            className={cn(
              "h-auto w-full justify-start gap-3 rounded-lg px-3 py-2.5 text-sm",
              customGender
                ? "bg-primary/10 text-primary font-medium hover:bg-primary/10 hover:text-primary"
                : "bg-secondary/20 text-foreground hover:bg-secondary/60",
            )}
            onClick={() => {
              onCustomGenderChange(true)
              if (!customGender) update({ gender: "" })
            }}
          >
            <span className="flex-1 text-left">{t("settings.profileGenderCustom")}</span>
            {customGender && <Check className="h-4 w-4 text-primary shrink-0" />}
          </Button>
        </div>
        {customGender && (
          <Input
            className="mt-2 bg-secondary/40 rounded-lg"
            {...textInputProps("gender")}
            placeholder={t("settings.profileGenderCustomPlaceholder")}
          />
        )}
      </div>

      {/* ── Language ── */}
      <div>
        <div className="text-xs font-medium text-muted-foreground mb-2 px-1">
          {t("settings.profileLanguage")}
        </div>
        <div className="space-y-0.5">
          <Button
            variant="ghost"
            className={cn(
              "h-auto w-full justify-start gap-3 rounded-lg px-3 py-2.5 text-sm",
              !config.language
                ? "bg-primary/10 text-primary font-medium hover:bg-primary/10 hover:text-primary"
                : "bg-secondary/20 text-foreground hover:bg-secondary/60",
            )}
            onClick={() => update({ language: null })}
          >
            <span className="flex-1 text-left">{t("settings.profileLanguageSystem")}</span>
            {!config.language && <Check className="h-4 w-4 text-primary shrink-0" />}
          </Button>
        </div>
        <Select
          value={config.language ?? ""}
          onValueChange={(v) => update({ language: v || null })}
        >
          <SelectTrigger className="mt-1 bg-secondary/20 text-sm hover:bg-secondary/60">
            <SelectValue placeholder={t("settings.profileLanguageSystem")} />
          </SelectTrigger>
          <SelectContent>
            {LANGUAGE_OPTIONS.map((lang) => (
              <SelectItem key={lang.code} value={lang.code}>
                {lang.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <p className="text-[11px] text-muted-foreground mt-1.5 px-1">
          {t("settings.profileLanguageHint")}
        </p>
      </div>

      <div className="border-t border-border/50" />

      {/* ── Response Style ── */}
      <div>
        <div className="text-xs font-medium text-muted-foreground mb-2 px-1">
          {t("settings.profileResponseStyle")}
        </div>
        <div className="space-y-0.5">
          {PRESET_STYLES.map((style) => (
            <Button
              key={style}
              variant="ghost"
              className={cn(
                "h-auto w-full justify-start gap-3 rounded-lg px-3 py-2.5 text-sm",
                !customStyle && config.responseStyle === style
                  ? "bg-primary/10 text-primary font-medium hover:bg-primary/10 hover:text-primary"
                  : "bg-secondary/20 text-foreground hover:bg-secondary/60",
              )}
              onClick={() => {
                onCustomStyleChange(false)
                update({ responseStyle: config.responseStyle === style ? null : style })
              }}
            >
              <span className="flex-1 text-left">
                {t(`settings.profileStyle${style.charAt(0).toUpperCase() + style.slice(1)}`)}
              </span>
              {!customStyle && config.responseStyle === style && (
                <Check className="h-4 w-4 text-primary shrink-0" />
              )}
            </Button>
          ))}
          <Button
            variant="ghost"
            className={cn(
              "h-auto w-full justify-start gap-3 rounded-lg px-3 py-2.5 text-sm",
              customStyle
                ? "bg-primary/10 text-primary font-medium hover:bg-primary/10 hover:text-primary"
                : "bg-secondary/20 text-foreground hover:bg-secondary/60",
            )}
            onClick={() => {
              onCustomStyleChange(true)
              if (!customStyle) update({ responseStyle: "" })
            }}
          >
            <span className="flex-1 text-left">{t("settings.profileStyleCustom")}</span>
            {customStyle && <Check className="h-4 w-4 text-primary shrink-0" />}
          </Button>
        </div>

        {customStyle && (
          <Textarea
            className="mt-2 bg-secondary/40 rounded-lg resize-none leading-relaxed"
            rows={4}
            {...textInputProps("responseStyle")}
            placeholder={t("settings.profileStyleCustomPlaceholder")}
          />
        )}
      </div>
    </>
  )
}
