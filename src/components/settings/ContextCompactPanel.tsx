import { useState, useEffect, useCallback } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { Switch } from "@/components/ui/switch"
import { Input } from "@/components/ui/input"
import { Button } from "@/components/ui/button"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
  IconTip,
} from "@/components/ui/tooltip"
import { ChevronDown, ChevronRight, Loader2, Check, Info } from "lucide-react"
import { cn } from "@/lib/utils"
import { logger } from "@/lib/logger"

interface CompactConfig {
  enabled: boolean
  toolPolicies: Record<string, string>
  maxToolResultContextShare: number
  softTrimRatio: number
  hardClearRatio: number
  keepLastAssistants: number
  minPrunableToolChars: number
  softTrimMaxChars: number
  softTrimHeadChars: number
  softTrimTailChars: number
  hardClearEnabled: boolean
  hardClearPlaceholder: string
  summarizationThreshold: number
  preserveRecentTurns: number
  identifierPolicy: string
  identifierInstructions: string | null
  customInstructions: string | null
  summarizationTimeoutSecs: number
  summaryMaxTokens: number
  maxHistoryShare: number
  maxCompactionSummaryChars: number
}

function RatioInput({
  label,
  desc,
  value,
  min,
  max,
  step,
  onChange,
}: {
  label: string
  desc: string
  value: number
  min: number
  max: number
  step: number
  onChange: (v: number) => void
}) {
  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between">
        <label className="text-sm">{label}</label>
        <span className="text-xs font-mono text-muted-foreground">{Math.round(value * 100)}%</span>
      </div>
      <input
        type="range"
        min={min * 100}
        max={max * 100}
        step={step * 100}
        value={value * 100}
        onChange={(e) => onChange(Number(e.target.value) / 100)}
        className="w-full h-1.5 bg-secondary rounded-full appearance-none cursor-pointer accent-primary"
      />
      <p className="text-[10px] text-muted-foreground/60">{desc}</p>
    </div>
  )
}

function NumberField({
  label,
  desc,
  value,
  min,
  max,
  onChange,
}: {
  label: string
  desc?: string
  value: number
  min: number
  max: number
  onChange: (v: number) => void
}) {
  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between gap-2">
        <label className="text-sm">{label}</label>
        <Input
          type="number"
          min={min}
          max={max}
          className="h-7 w-24 text-sm text-right"
          value={value}
          onChange={(e) => {
            const v = Number(e.target.value)
            if (!isNaN(v)) onChange(Math.max(min, Math.min(max, v)))
          }}
        />
      </div>
      {desc && <p className="text-[10px] text-muted-foreground/60">{desc}</p>}
    </div>
  )
}

export default function ContextCompactPanel() {
  const { t } = useTranslation()
  const [config, setConfig] = useState<CompactConfig | null>(null)
  const [savedJson, setSavedJson] = useState("")
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  const [pruningOpen, setPruningOpen] = useState(true)
  const [summaryOpen, setSummaryOpen] = useState(true)
  const [advancedOpen, setAdvancedOpen] = useState(false)
  const [availableTools, setAvailableTools] = useState<{ name: string; description: string }[]>([])

  useEffect(() => {
    getTransport().call<CompactConfig>("get_compact_config")
      .then((c) => {
        setConfig(c)
        setSavedJson(JSON.stringify(c))
      })
      .catch((e) =>
        logger.error("settings", "ContextCompactPanel::load", "Failed to load compact config", e),
      )
    getTransport().call<{ name: string; description: string }[]>("list_builtin_tools")
      .then(setAvailableTools)
      .catch(() => { })
  }, [])

  const isDirty = config ? JSON.stringify(config) !== savedJson : false

  const handleSave = useCallback(async () => {
    if (!config) return
    setSaving(true)
    try {
      await getTransport().call("save_compact_config", { config })
      setSavedJson(JSON.stringify(config))
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "ContextCompactPanel::save", "Failed to save compact config", e)
      setSaveStatus("failed")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } finally {
      setSaving(false)
    }
  }, [config])

  const update = useCallback((patch: Partial<CompactConfig>) => {
    setConfig((prev) => (prev ? { ...prev, ...patch } : prev))
  }, [])

  if (!config) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
      </div>
    )
  }

  return (
    <div className="space-y-4">
      <div className="border-t border-border/30 pt-4 mt-2">
        <h3 className="text-sm font-medium mb-1">{t("settings.contextCompact")}</h3>
        <p className="text-xs text-muted-foreground">{t("settings.contextCompactDesc")}</p>
      </div>

      {/* Global toggle */}
      <div
        className="flex items-center justify-between px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors cursor-pointer"
        onClick={() => update({ enabled: !config.enabled })}
      >
        <div className="space-y-0.5">
          <div className="text-sm font-medium">{t("settings.contextCompactEnabled")}</div>
          <div className="text-xs text-muted-foreground">
            {t("settings.contextCompactEnabledDesc")}
          </div>
        </div>
        <Switch checked={config.enabled} onCheckedChange={(v) => update({ enabled: v })} />
      </div>

      {config.enabled && (
        <>
          {/* ── Tool Compact Policy ── */}
          <div className="rounded-lg border border-border/50 bg-secondary/20 overflow-hidden">
            <button
              className="flex items-center gap-2 px-3 py-2.5 w-full text-left hover:bg-secondary/30 transition-colors"
              onClick={() => setPruningOpen(!pruningOpen)}
            >
              {pruningOpen ? (
                <ChevronDown className="h-3.5 w-3.5" />
              ) : (
                <ChevronRight className="h-3.5 w-3.5" />
              )}
              <span className="text-sm font-medium">{t("settings.contextCompactToolPolicy")}</span>
            </button>
            {pruningOpen && (
              <div className="px-3 pb-3 pt-1 space-y-3 border-t border-border/30">
                <p className="text-xs text-muted-foreground">{t("settings.contextCompactToolPolicyDesc")}</p>
                {/* Tool list */}
                <div className="rounded-lg border border-border/50 overflow-hidden">
                  {availableTools.map((tool, idx) => {
                    const policies = config.toolPolicies || {}
                    const policy = (policies[tool.name] === "eager" || policies[tool.name] === "protect")
                      ? policies[tool.name]
                      : "default"
                    const displayName =
                      t(`tools.${tool.name}`, { defaultValue: "" }) || tool.name
                    return (
                      <div
                        key={tool.name}
                        className={cn(
                          "flex items-center justify-between px-3 py-2 gap-3",
                          idx > 0 && "border-t border-border/30",
                        )}
                      >
                        <Tooltip>
                          <TooltipTrigger asChild>
                            <span className="text-xs truncate flex-1 min-w-0 cursor-default">{displayName}</span>
                          </TooltipTrigger>
                          <TooltipContent side="top">
                            <span className="font-mono text-[10px]">{tool.name}</span>
                          </TooltipContent>
                        </Tooltip>
                        <select
                          className={cn(
                            "h-6 rounded border border-border bg-background px-1.5 text-[11px] shrink-0",
                            policy === "eager" && "text-red-600 dark:text-red-400",
                            policy === "protect" && "text-green-600 dark:text-green-400",
                          )}
                          value={policy}
                          onChange={(e) => {
                            const next = { ...policies }
                            if (e.target.value === "default") {
                              delete next[tool.name]
                            } else {
                              next[tool.name] = e.target.value
                            }
                            update({ toolPolicies: next })
                          }}
                        >
                          <option value="default">{t("settings.contextCompactPolicyDefault")}</option>
                          <option value="eager">{t("settings.contextCompactPolicyEager")}</option>
                          <option value="protect">{t("settings.contextCompactPolicyProtect")}</option>
                        </select>
                      </div>
                    )
                  })}
                </div>
                <div className="border-t border-border/30 pt-3 space-y-3">
                  <RatioInput
                    label={t("settings.contextCompactMaxToolResultShare")}
                    desc={t("settings.contextCompactMaxToolResultShareDesc")}
                    value={config.maxToolResultContextShare}
                    min={0.1}
                    max={0.6}
                    step={0.05}
                    onChange={(v) => update({ maxToolResultContextShare: v })}
                  />
                  <RatioInput
                    label={t("settings.contextCompactSoftTrimRatio")}
                    desc={t("settings.contextCompactSoftTrimRatioDesc")}
                    value={config.softTrimRatio}
                    min={0.1}
                    max={0.8}
                    step={0.05}
                    onChange={(v) => update({ softTrimRatio: v })}
                  />
                  <RatioInput
                    label={t("settings.contextCompactHardClearRatio")}
                    desc={t("settings.contextCompactHardClearRatioDesc")}
                    value={config.hardClearRatio}
                    min={0.2}
                    max={0.9}
                    step={0.05}
                    onChange={(v) => update({ hardClearRatio: v })}
                  />
                  <NumberField
                    label={t("settings.contextCompactKeepAssistants")}
                    desc={t("settings.contextCompactKeepAssistantsDesc")}
                    value={config.keepLastAssistants}
                    min={1}
                    max={10}
                    onChange={(v) => update({ keepLastAssistants: v })}
                  />
                </div>
              </div>
            )}
          </div>

          {/* ── Summarization Section ── */}
          <div className="rounded-lg border border-border/50 bg-secondary/20 overflow-hidden">
            <button
              className="flex items-center gap-2 px-3 py-2.5 w-full text-left hover:bg-secondary/30 transition-colors"
              onClick={() => setSummaryOpen(!summaryOpen)}
            >
              {summaryOpen ? (
                <ChevronDown className="h-3.5 w-3.5" />
              ) : (
                <ChevronRight className="h-3.5 w-3.5" />
              )}
              <span className="text-sm font-medium">
                {t("settings.contextCompactSummarization")}
              </span>
            </button>
            {summaryOpen && (
              <div className="px-3 pb-3 pt-1 space-y-3 border-t border-border/30">
                <RatioInput
                  label={t("settings.contextCompactSummarizationThreshold")}
                  desc={t("settings.contextCompactSummarizationThresholdDesc")}
                  value={config.summarizationThreshold}
                  min={0.5}
                  max={0.95}
                  step={0.05}
                  onChange={(v) => update({ summarizationThreshold: v })}
                />
                <NumberField
                  label={t("settings.contextCompactPreserveTurns")}
                  desc={t("settings.contextCompactPreserveTurnsDesc")}
                  value={config.preserveRecentTurns}
                  min={1}
                  max={12}
                  onChange={(v) => update({ preserveRecentTurns: v })}
                />
                <div className="flex items-center justify-between gap-2">
                  <label className="text-sm">
                    {t("settings.contextCompactIdentifierPolicy")}
                  </label>
                  <select
                    className="h-7 w-32 rounded-md border border-border bg-background px-2 text-sm text-right"
                    value={config.identifierPolicy}
                    onChange={(e) => update({ identifierPolicy: e.target.value })}
                  >
                    <option value="strict">
                      {t("settings.contextCompactIdentifierPolicyStrict")}
                    </option>
                    <option value="off">{t("settings.contextCompactIdentifierPolicyOff")}</option>
                    <option value="custom">
                      {t("settings.contextCompactIdentifierPolicyCustom")}
                    </option>
                  </select>
                </div>
                {config.identifierPolicy === "custom" && (
                  <div className="space-y-1">
                    <label className="text-sm">{t("settings.contextCompactIdentifierInstructions")}</label>
                    <textarea
                      className="w-full rounded-md border border-border bg-background px-2 py-1.5 text-sm min-h-[60px] resize-y"
                      placeholder={t("settings.contextCompactIdentifierInstructionsPlaceholder")}
                      value={config.identifierInstructions ?? ""}
                      onChange={(e) =>
                        update({ identifierInstructions: e.target.value || null })
                      }
                    />
                  </div>
                )}
                <NumberField
                  label={t("settings.contextCompactTimeout")}
                  value={config.summarizationTimeoutSecs}
                  min={10}
                  max={300}
                  onChange={(v) => update({ summarizationTimeoutSecs: v })}
                />
                <NumberField
                  label={t("settings.contextCompactMaxSummaryChars")}
                  desc={t("settings.contextCompactMaxSummaryCharsDesc")}
                  value={config.maxCompactionSummaryChars}
                  min={4000}
                  max={64000}
                  onChange={(v) => update({ maxCompactionSummaryChars: v })}
                />
              </div>
            )}
          </div>

          {/* ── Advanced Section ── */}
          <div className="rounded-lg border border-border/50 bg-secondary/20 overflow-hidden">
            <button
              className="flex items-center gap-2 px-3 py-2.5 w-full text-left hover:bg-secondary/30 transition-colors"
              onClick={() => setAdvancedOpen(!advancedOpen)}
            >
              {advancedOpen ? (
                <ChevronDown className="h-3.5 w-3.5" />
              ) : (
                <ChevronRight className="h-3.5 w-3.5" />
              )}
              <span className="text-sm font-medium">{t("settings.contextCompactAdvanced")}</span>
            </button>
            {advancedOpen && (
              <div className="px-3 pb-3 pt-1 space-y-3 border-t border-border/30">
                <NumberField
                  label={t("settings.contextCompactSoftTrimMaxChars")}
                  desc={t("settings.contextCompactSoftTrimMaxCharsDesc")}
                  value={config.softTrimMaxChars}
                  min={1000}
                  max={50000}
                  onChange={(v) => update({ softTrimMaxChars: v })}
                />
                <NumberField
                  label={t("settings.contextCompactHeadChars")}
                  value={config.softTrimHeadChars}
                  min={500}
                  max={10000}
                  onChange={(v) => update({ softTrimHeadChars: v })}
                />
                <NumberField
                  label={t("settings.contextCompactTailChars")}
                  value={config.softTrimTailChars}
                  min={500}
                  max={10000}
                  onChange={(v) => update({ softTrimTailChars: v })}
                />
                <NumberField
                  label={t("settings.contextCompactMinPrunableChars")}
                  desc={t("settings.contextCompactMinPrunableCharsDesc")}
                  value={config.minPrunableToolChars}
                  min={1000}
                  max={200000}
                  onChange={(v) => update({ minPrunableToolChars: v })}
                />
                <NumberField
                  label={t("settings.contextCompactMaxHistoryShare")}
                  value={Math.round(config.maxHistoryShare * 100)}
                  min={10}
                  max={90}
                  onChange={(v) => update({ maxHistoryShare: v / 100 })}
                />
                <div className="flex items-center justify-between px-0 py-1">
                  <label className="text-sm">
                    {t("settings.contextCompactHardClearEnabled") || "Hard clear enabled"}
                  </label>
                  <Switch
                    checked={config.hardClearEnabled}
                    onCheckedChange={(v) => update({ hardClearEnabled: v })}
                  />
                </div>
                <div className="pt-3">
                  <label className="text-sm block mb-2">
                    {t("settings.contextCompactCustomInstructions")}
                  </label>
                  <textarea
                    className="w-full rounded-md border border-border bg-background px-2 py-1.5 text-sm min-h-[60px] resize-y"
                    placeholder={t("settings.contextCompactCustomInstructions")}
                    value={config.customInstructions ?? ""}
                    onChange={(e) =>
                      update({
                        customInstructions: e.target.value || null,
                      })
                    }
                  />
                </div>
              </div>
            )}
          </div>
        </>
      )}

      {/* Save button */}
      <div className="flex items-center gap-2 pt-2">
        <Button
          variant="default"
          size="sm"
          onClick={handleSave}
          disabled={(!isDirty && saveStatus === "idle") || saving}
          className={cn(
            saveStatus === "saved" && "bg-green-500/10 text-green-600 hover:bg-green-500/20",
            saveStatus === "failed" && "bg-destructive/10 text-destructive hover:bg-destructive/20",
          )}
        >
          {saving ? (
            <span className="flex items-center gap-1.5">
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              {t("common.saving")}
            </span>
          ) : saveStatus === "saved" ? (
            <span className="flex items-center gap-1.5">
              <Check className="h-3.5 w-3.5" />
              {t("common.saved")}
            </span>
          ) : saveStatus === "failed" ? (
            t("common.saveFailed")
          ) : (
            t("common.save")
          )}
        </Button>
      </div>
    </div>
  )
}
