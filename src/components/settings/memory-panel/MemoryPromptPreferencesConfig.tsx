import { useCallback, useEffect, useId, useState } from "react"
import { useTranslation } from "react-i18next"
import { Check, Loader2, Save } from "lucide-react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { Button } from "@/components/ui/button"
import { RadioPills } from "@/components/ui/radio-pills"
import { Switch } from "@/components/ui/switch"
import { Textarea } from "@/components/ui/textarea"
import type { MemoryRuntimeConfig } from "./types"

import {
  DEFAULT_MEMORY_PROMPT_PREFERENCES,
  type MemoryPromptPreference,
  type MemoryPromptStage,
  type MemoryPromptOverrides,
} from "./memoryPromptPreferences"

interface Props {
  overrides?: MemoryPromptOverrides
  onAgentChange?: (value: MemoryPromptOverrides) => void
}

export default function MemoryPromptPreferencesConfig({ overrides, onAgentChange }: Props) {
  const { t } = useTranslation()
  const id = useId()
  const [global, setGlobal] = useState(DEFAULT_MEMORY_PROMPT_PREFERENCES)
  const [original, setOriginal] = useState(DEFAULT_MEMORY_PROMPT_PREFERENCES)
  const [loaded, setLoaded] = useState(false)
  const [error, setError] = useState(false)
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  const isAgent = onAgentChange !== undefined

  const load = useCallback(async () => {
    try {
      const config = await getTransport().call<MemoryRuntimeConfig>("get_memory_runtime_config")
      const value = config.promptPreferences ?? DEFAULT_MEMORY_PROMPT_PREFERENCES
      setGlobal(value)
      setOriginal(value)
      setLoaded(true)
      setError(false)
    } catch (e) {
      logger.error("settings", "MemoryPromptPreferencesConfig::load", "Failed to load preferences", e)
      setError(true)
    }
  }, [])

  useEffect(() => { void load() }, [load])
  useEffect(() => {
    if (saveStatus === "idle") return
    const timer = setTimeout(() => setSaveStatus("idle"), 2000)
    return () => clearTimeout(timer)
  }, [saveStatus])

  const save = async () => {
    setSaving(true)
    try {
      // Read the current document immediately before saving this section so
      // unrelated budget/consent edits made in another panel are preserved.
      const current = await getTransport().call<MemoryRuntimeConfig>("get_memory_runtime_config")
      const saved = await getTransport().call<MemoryRuntimeConfig>("save_memory_runtime_config", {
        config: { ...current, promptPreferences: global },
      })
      setGlobal(saved.promptPreferences ?? global)
      setOriginal(saved.promptPreferences ?? global)
      setError(false)
      setSaveStatus("saved")
    } catch (e) {
      logger.error("settings", "MemoryPromptPreferencesConfig::save", "Failed to save preferences", e)
      setSaveStatus("failed")
    } finally {
      setSaving(false)
    }
  }

  const update = (stage: MemoryPromptStage, value: MemoryPromptPreference | null) => {
    if (onAgentChange) {
      onAgentChange({ ...overrides, [stage]: value })
    } else if (value) {
      setGlobal((previous) => ({ ...previous, [stage]: value }))
    }
  }

  return (
    <div className="space-y-4 rounded-lg bg-secondary/30 p-3">
      <div>
        <h3 className="text-sm font-medium">{t("settings.memoryPromptPreferences.title")}</h3>
        <p className="mt-1 text-xs text-muted-foreground">{t("settings.memoryPromptPreferences.desc")}</p>
      </div>
      {error && (
        <div className="flex items-center gap-2 text-xs text-destructive">
          <span>{t("settings.memoryPromptPreferences.error")}</span>
          <Button type="button" size="sm" variant="ghost" onClick={() => void load()}>
            {t("settings.memoryPromptPreferences.retry")}
          </Button>
        </div>
      )}
      {!loaded && !error && <Loader2 className="size-4 animate-spin" />}
      {loaded && (["extraction", "profile", "dreaming"] as const).map((stage) => {
        const inherited = isAgent && overrides?.[stage] == null
        const value = (isAgent ? overrides?.[stage] : global[stage]) ?? global[stage]
        const disabled = saving || inherited
        return (
          <div key={stage} className="space-y-2">
            <div className="flex items-center justify-between gap-2">
              <h4 className="text-xs font-medium">{t(`settings.memoryPromptPreferences.${stage}`)}</h4>
              {isAgent && (
                <label className="flex items-center gap-2 text-xs text-muted-foreground">
                  {t("settings.memoryPromptPreferences.inherit")}
                  <Switch
                    checked={inherited}
                    onCheckedChange={(checked) => update(stage, checked ? null : { ...global[stage] })}
                    aria-label={`${t(`settings.memoryPromptPreferences.${stage}`)}: ${t("settings.memoryPromptPreferences.inherit")}`}
                  />
                </label>
              )}
            </div>
            <RadioPills
              value={value.style}
              options={(["default", "concise", "detailed"] as const).map((style) => ({
                value: style,
                label: t(`settings.memoryPromptPreferences.${style}`),
                disabled,
              }))}
              onChange={(style) => update(stage, { ...value, style })}
              ariaLabel={t(`settings.memoryPromptPreferences.${stage}`)}
            />
            <label htmlFor={`${id}-${stage}`} className="block text-xs text-muted-foreground">
              {t("settings.memoryPromptPreferences.supplemental")}
            </label>
            <Textarea
              id={`${id}-${stage}`}
              value={value.supplemental}
              disabled={disabled}
              maxLength={512}
              rows={2}
              onChange={(event) => update(stage, { ...value, supplemental: event.target.value })}
            />
            {(stage === "profile" || stage === "dreaming") && (
              <p className="text-[11px] text-muted-foreground">
                {t(`settings.memoryPromptPreferences.${stage}Note`)}
              </p>
            )}
          </div>
        )
      })}
      {!isAgent && loaded && (
        <Button
          type="button"
          size="sm"
          disabled={saving || JSON.stringify(global) === JSON.stringify(original)}
          className={saveStatus === "saved" ? "text-green-600" : saveStatus === "failed" ? "text-destructive" : ""}
          onClick={() => void save()}
        >
          {saving ? <Loader2 className="size-3.5 animate-spin" /> : saveStatus === "saved" ? <Check className="size-3.5" /> : <Save className="size-3.5" />}
          {t(`settings.memoryPromptPreferences.${saving ? "saving" : saveStatus === "saved" ? "saved" : saveStatus === "failed" ? "error" : "save"}`)}
        </Button>
      )}
    </div>
  )
}
