import { useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Loader2, Check, AlertTriangle } from "lucide-react"

import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"

// ── Types ─────────────────────────────────────────────────────────

type AwarenessMode = "off" | "structured" | "llm_digest"

interface ExtractionModelRef {
  providerId: string
  model: string
}

interface LlmExtractionConfig {
  extractionAgent: string | null
  extractionModel: ExtractionModelRef | null
  minIntervalSecs: number
  maxCandidates: number
  digestMaxChars: number
  concurrency: number
  perSessionInputChars: number
  inputLookbackHours: number
  fallbackOnError: boolean
  reuseSideQueryCache: boolean
}

interface AwarenessConfig {
  enabled: boolean
  mode: AwarenessMode
  maxSessions: number
  maxChars: number
  lookbackHours: number
  activeWindowSecs: number
  sameAgentOnly: boolean
  excludeCron: boolean
  excludeChannel: boolean
  excludeSubagents: boolean
  previewChars: number
  dynamicEnabled: boolean
  minRefreshSecs: number
  semanticHintRegex: string
  refreshOnCompaction: boolean
  llmExtraction: LlmExtractionConfig
}

type SaveStatus = "idle" | "saved" | "failed"

// ── Component ─────────────────────────────────────────────────────

export default function AwarenessPanel() {
  const { t } = useTranslation()
  const [cfg, setCfg] = useState<AwarenessConfig | null>(null)
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<SaveStatus>("idle")

  useEffect(() => {
    getTransport()
      .call<AwarenessConfig>("get_awareness_config")
      .then((c) => {
        setCfg(c)
        setLoading(false)
      })
      .catch((e: unknown) => {
        logger.error("settings", "AwarenessPanel::load", "Failed to load config", e)
        setLoading(false)
      })
  }, [])

  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  const save = (next: AwarenessConfig) => {
    setCfg(next)
    // Debounce: wait 500ms after the last change before persisting.
    if (saveTimer.current) clearTimeout(saveTimer.current)
    saveTimer.current = setTimeout(async () => {
      setSaving(true)
      try {
        await getTransport().call("save_awareness_config", {
          config: next,
        })
        setSaveStatus("saved")
        setTimeout(() => setSaveStatus("idle"), 1500)
      } catch (e) {
        logger.error("settings", "AwarenessPanel::save", "Failed to save awareness config", e)
        setSaveStatus("failed")
        setTimeout(() => setSaveStatus("idle"), 1500)
        // Rollback: re-fetch actual backend state so UI stays in sync.
        try {
          const fresh = await getTransport().call<AwarenessConfig>("get_awareness_config")
          setCfg(fresh)
        } catch {
          /* best effort */
        }
      } finally {
        setSaving(false)
      }
    }, 500)
  }

  if (loading || !cfg) return null

  const disabled = !cfg.enabled

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <div className="text-sm font-medium">
            {t("settings.awareness.title", "Behavior Awareness")}
          </div>
          <div className="text-xs text-muted-foreground">
            {t(
              "settings.awareness.desc",
              "Give this chat a dynamic view of what the user is doing in other parallel sessions.",
            )}
          </div>
        </div>
        <div className="flex items-center gap-2">
          {saving && <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />}
          {saveStatus === "saved" && <Check className="h-4 w-4 text-emerald-500" />}
          <Switch checked={cfg.enabled} onCheckedChange={(v) => save({ ...cfg, enabled: v })} />
        </div>
      </div>

      <div className={disabled ? "pointer-events-none opacity-50" : ""}>
        {/* Mode selector */}
        <div className="space-y-1">
          <label className="text-xs font-medium">{t("settings.awareness.mode", "Mode")}</label>
          <Select
            value={cfg.mode}
            onValueChange={(v: string) => save({ ...cfg, mode: v as AwarenessMode })}
          >
            <SelectTrigger className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="off">
                {t("settings.awareness.modeOff", "Off (feature disabled)")}
              </SelectItem>
              <SelectItem value="structured">
                {t("settings.awareness.modeStructured", "Structured (zero LLM cost, default)")}
              </SelectItem>
              <SelectItem value="llm_digest">
                {t("settings.awareness.modeLlm", "LLM Digest (extra API cost)")}
              </SelectItem>
            </SelectContent>
          </Select>
        </div>

        {cfg.mode === "llm_digest" && (
          <div className="mt-3 flex items-start gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 p-3 text-xs">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-amber-500" />
            <div>
              {t(
                "settings.awareness.llmWarning",
                "LLM Digest mode runs an extra side_query per user turn (throttled to min_interval_secs). Expect extra API cost.",
              )}
            </div>
          </div>
        )}

        {/* Scope */}
        <div className="mt-4 grid grid-cols-2 gap-3">
          <NumField
            label={t("settings.awareness.maxSessions", "Max sessions")}
            value={cfg.maxSessions}
            onChange={(v) => save({ ...cfg, maxSessions: v })}
          />
          <NumField
            label={t("settings.awareness.lookbackHours", "Lookback (hours)")}
            value={cfg.lookbackHours}
            onChange={(v) => save({ ...cfg, lookbackHours: v })}
          />
        </div>

        {/* Session-type toggles (positive framing) */}
        <div className="mt-4 space-y-2">
          <div className="text-xs font-medium text-muted-foreground">
            {t("settings.awareness.includeTypes", "Session types to include")}
          </div>
          <LabeledSwitch
            label={t("settings.awareness.sameAgentOnly", "Same agent only")}
            value={cfg.sameAgentOnly}
            onChange={(v) => save({ ...cfg, sameAgentOnly: v })}
          />
          <LabeledSwitch
            label={t("settings.awareness.includeCron", "Include cron sessions")}
            value={!cfg.excludeCron}
            onChange={(v) => save({ ...cfg, excludeCron: !v })}
          />
          <LabeledSwitch
            label={t("settings.awareness.includeChannel", "Include IM channel sessions")}
            value={!cfg.excludeChannel}
            onChange={(v) => save({ ...cfg, excludeChannel: !v })}
          />
          <LabeledSwitch
            label={t("settings.awareness.includeSubagents", "Include sub-agent sessions")}
            value={!cfg.excludeSubagents}
            onChange={(v) => save({ ...cfg, excludeSubagents: !v })}
          />
        </div>

        {/* Refresh */}
        <div className="mt-4 space-y-2">
          <div className="text-xs font-medium text-muted-foreground">
            {t("settings.awareness.refresh", "Dynamic refresh")}
          </div>
          <LabeledSwitch
            label={t("settings.awareness.dynamicEnabled", "Refresh suffix every turn")}
            value={cfg.dynamicEnabled}
            onChange={(v) => save({ ...cfg, dynamicEnabled: v })}
          />
          <NumField
            label={t("settings.awareness.minRefreshSecs", "Min refresh interval (seconds)")}
            value={cfg.minRefreshSecs}
            onChange={(v) => save({ ...cfg, minRefreshSecs: v })}
          />
        </div>

        {/* LLM extraction */}
        {cfg.mode === "llm_digest" && (
          <div className="mt-4 space-y-2 rounded-md border border-border/40 bg-muted/30 p-3">
            <div className="text-xs font-medium text-muted-foreground">
              {t("settings.awareness.llmExtraction", "LLM Extraction")}
            </div>
            <NumField
              label={t(
                "settings.awareness.minIntervalSecs",
                "Min interval between extractions (seconds)",
              )}
              value={cfg.llmExtraction.minIntervalSecs}
              onChange={(v) =>
                save({
                  ...cfg,
                  llmExtraction: { ...cfg.llmExtraction, minIntervalSecs: v },
                })
              }
            />
            <NumField
              label={t("settings.awareness.maxCandidates", "Max candidate sessions")}
              value={cfg.llmExtraction.maxCandidates}
              onChange={(v) =>
                save({
                  ...cfg,
                  llmExtraction: { ...cfg.llmExtraction, maxCandidates: v },
                })
              }
            />
            <NumField
              label={t("settings.awareness.inputLookbackHours", "Input lookback (hours)")}
              value={cfg.llmExtraction.inputLookbackHours}
              onChange={(v) =>
                save({
                  ...cfg,
                  llmExtraction: { ...cfg.llmExtraction, inputLookbackHours: v },
                })
              }
            />
            <NumField
              label={t("settings.awareness.digestMaxChars", "Digest output budget (chars)")}
              value={cfg.llmExtraction.digestMaxChars}
              onChange={(v) =>
                save({
                  ...cfg,
                  llmExtraction: { ...cfg.llmExtraction, digestMaxChars: v },
                })
              }
            />
            <div>
              <label className="text-xs font-medium">
                {t("settings.awareness.extractionAgent", "Extraction agent ID (optional)")}
              </label>
              <Input
                value={cfg.llmExtraction.extractionAgent ?? ""}
                placeholder={t(
                  "settings.awareness.extractionAgentPlaceholder",
                  "Leave empty to inherit recap.analysisAgent",
                )}
                onChange={(e) =>
                  save({
                    ...cfg,
                    llmExtraction: {
                      ...cfg.llmExtraction,
                      extractionAgent: e.target.value || null,
                    },
                  })
                }
              />
            </div>
          </div>
        )}

        <div className="mt-4 text-xs text-muted-foreground">
          {t(
            "settings.awareness.perSessionHint",
            "Each chat can override these settings via the eye icon in the input bar.",
          )}
        </div>

        <div className="mt-4 flex justify-end">
          <Button
            variant="outline"
            size="sm"
            onClick={async () => {
              try {
                const fresh = await getTransport().call<AwarenessConfig>("get_awareness_config")
                setCfg(fresh)
              } catch (e) {
                logger.error("settings", "AwarenessPanel::reload", "Failed to reload config", e)
              }
            }}
          >
            {t("settings.awareness.reloadDefaults", "Reload")}
          </Button>
        </div>
      </div>
    </div>
  )
}

function LabeledSwitch({
  label,
  value,
  onChange,
}: {
  label: string
  value: boolean
  onChange: (v: boolean) => void
}) {
  return (
    <div className="flex items-center justify-between py-1">
      <span className="text-sm">{label}</span>
      <Switch checked={value} onCheckedChange={onChange} />
    </div>
  )
}

function NumField({
  label,
  value,
  onChange,
}: {
  label: string
  value: number
  onChange: (v: number) => void
}) {
  return (
    <div className="space-y-1">
      <label className="text-xs font-medium">{label}</label>
      <Input
        type="number"
        value={value}
        onChange={(e) => {
          const v = Number(e.target.value)
          if (!Number.isNaN(v) && v >= 0) {
            onChange(v)
          }
        }}
      />
    </div>
  )
}
