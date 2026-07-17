import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Check, Loader2 } from "lucide-react"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import { DeferredNumberInput } from "@/components/ui/deferred-number-input"
import {
  DEFAULT_KNOWLEDGE_SOURCE_LIMITS,
  MAX_KNOWLEDGE_BINARY_SOURCE_MB,
  MAX_KNOWLEDGE_TEXT_SOURCE_MB,
  MAX_KNOWLEDGE_URL_RESPONSE_MB,
  MIN_KNOWLEDGE_BINARY_SOURCE_MB,
  MIN_KNOWLEDGE_TEXT_SOURCE_MB,
  MIN_KNOWLEDGE_URL_RESPONSE_MB,
  useKnowledgeSourceLimits,
} from "@/lib/knowledgeSourceLimits"
import SettingsResetControl from "./SettingsResetControl"

export default function KnowledgeSourceLimitsSection() {
  const { t } = useTranslation()
  const { config: loaded, refresh, save: saveLimits } = useKnowledgeSourceLimits()
  const [draft, setDraft] = useState(DEFAULT_KNOWLEDGE_SOURCE_LIMITS)
  const [saving, setSaving] = useState(false)
  const [saved, setSaved] = useState(false)

  useEffect(() => setDraft(loaded), [loaded])

  const dirty = JSON.stringify(draft) !== JSON.stringify(loaded)
  const save = async () => {
    setSaving(true)
    try {
      const next = await saveLimits(draft)
      setDraft(next)
      setSaved(true)
      window.setTimeout(() => setSaved(false), 1800)
    } catch (error) {
      toast.error(error instanceof Error ? error.message : String(error))
    } finally {
      setSaving(false)
    }
  }

  return (
    <section className="rounded-lg border border-border bg-card">
      <div className="flex items-start justify-between gap-3 px-4 py-3">
        <div>
          <div className="text-sm font-medium">{t("settings.knowledgeSourceLimits.title")}</div>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {t("settings.knowledgeSourceLimits.description")}
          </p>
        </div>
        <SettingsResetControl
          scope="knowledge"
          resetSection="source_limits"
          sectionLabel={t("settings.knowledgeSourceLimits.title")}
          level="region"
          onReset={refresh}
        />
      </div>
      <div className="grid gap-3 border-t border-border px-4 py-3 md:grid-cols-3">
        <LimitInput
          label={t("settings.knowledgeSourceLimits.text")}
          value={draft.maxTextSourceMb}
          min={MIN_KNOWLEDGE_TEXT_SOURCE_MB}
          max={MAX_KNOWLEDGE_TEXT_SOURCE_MB}
          onChange={(maxTextSourceMb) => setDraft((value) => ({ ...value, maxTextSourceMb }))}
        />
        <LimitInput
          label={t("settings.knowledgeSourceLimits.binary")}
          value={draft.maxBinarySourceMb}
          min={MIN_KNOWLEDGE_BINARY_SOURCE_MB}
          max={MAX_KNOWLEDGE_BINARY_SOURCE_MB}
          onChange={(maxBinarySourceMb) => setDraft((value) => ({ ...value, maxBinarySourceMb }))}
        />
        <LimitInput
          label={t("settings.knowledgeSourceLimits.url")}
          value={draft.maxUrlResponseMb}
          min={MIN_KNOWLEDGE_URL_RESPONSE_MB}
          max={MAX_KNOWLEDGE_URL_RESPONSE_MB}
          onChange={(maxUrlResponseMb) => setDraft((value) => ({ ...value, maxUrlResponseMb }))}
        />
      </div>
      <div className="flex justify-end gap-2 border-t border-border px-4 py-3">
        <Button size="sm" disabled={!dirty || saving} onClick={() => void save()}>
          {saving ? (
            <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
          ) : saved ? (
            <Check className="mr-1.5 h-3.5 w-3.5" />
          ) : null}
          {saved ? t("common.saved") : t("common.save")}
        </Button>
      </div>
    </section>
  )
}

function LimitInput({
  label,
  value,
  min,
  max,
  onChange,
}: {
  label: string
  value: number
  min: number
  max: number
  onChange: (value: number) => void
}) {
  return (
    <label className="space-y-1">
      <span className="text-xs font-medium">{label}</span>
      <div className="flex items-center gap-2">
        <DeferredNumberInput
          value={value}
          min={min}
          max={max}
          onValueCommit={onChange}
          className="h-8 flex-1 text-xs"
        />
        <span className="text-xs text-muted-foreground">MiB</span>
      </div>
      <span className="block text-[11px] text-muted-foreground">
        {min}–{max} MiB
      </span>
    </label>
  )
}
