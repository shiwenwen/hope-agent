import { useEffect, useState, useEffectEvent } from "react"
import { useTranslation } from "react-i18next"
import { getTransport } from "@/lib/transport-provider"
import { Textarea } from "@/components/ui/textarea"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { Button } from "@/components/ui/button"
import { Loader2, Check, Save } from "lucide-react"
import MemoryPanel from "@/components/settings/MemoryPanel"
import MemoryBudgetInputs from "@/components/settings/memory-panel/MemoryBudgetInputs"
import { logger } from "@/lib/logger"
import type {
  AgentConfig,
  ActiveMemoryConfig,
  AgentMemoryConfig,
  MemoryBudgetConfig,
} from "../types"
import { DEFAULT_ACTIVE_MEMORY, DEFAULT_MEMORY_BUDGET } from "../types"

const DEFAULT_AGENT_MEMORY: AgentMemoryConfig = {
  enabled: true,
  shared: true,
  promptBudget: 5000,
  activeMemory: DEFAULT_ACTIVE_MEMORY,
}

interface MemoryTabProps {
  agentId: string
  openclawMode?: boolean
  config: AgentConfig
  updateConfig: (patch: Partial<AgentConfig>) => void
}

export default function MemoryTab({ agentId, openclawMode, config, updateConfig }: MemoryTabProps) {
  const { t } = useTranslation()
  const [content, setContent] = useState("")
  const [originalContent, setOriginalContent] = useState("")
  const [loaded, setLoaded] = useState(false)
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")

  const loadContent = async () => {
    try {
      const md = await getTransport().call<string | null>("get_agent_memory_md", { id: agentId })
      const val = md ?? ""
      setContent(val)
      setOriginalContent(val)
      setLoaded(true)
    } catch (e) {
      logger.error("settings", "MemoryTab::loadCoreMemory", "Failed to load", e)
    }
  }
  const loadContentEffectEvent = useEffectEvent(loadContent)

  useEffect(() => {
    loadContentEffectEvent()
  }, [agentId])

  // Listen for updates from the agent tool
  useEffect(() => {
    const unlisten = getTransport().listen("core_memory_updated", (raw) => {
      const payload = raw as { agentId: string; scope: string }
      if (payload.scope === "agent" && payload.agentId === agentId) {
        loadContentEffectEvent()
      }
    })
    return unlisten
  }, [agentId])

  const handleSave = async () => {
    setSaving(true)
    try {
      await getTransport().call("save_agent_memory_md", { id: agentId, content })
      setOriginalContent(content)
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "MemoryTab::saveCoreMemory", "Failed to save", e)
      setSaveStatus("failed")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } finally {
      setSaving(false)
    }
  }

  const hasChanges = content !== originalContent

  const activeMemory: ActiveMemoryConfig = config.memory?.activeMemory ?? {
    ...DEFAULT_ACTIVE_MEMORY,
  }

  const updateActiveMemory = (patch: Partial<ActiveMemoryConfig>) => {
    const prevMemory = config.memory ?? DEFAULT_AGENT_MEMORY
    updateConfig({
      memory: {
        ...prevMemory,
        activeMemory: { ...activeMemory, ...patch },
      },
    })
  }

  const useGlobalBudget = !config.memory?.budget
  const budgetValue: MemoryBudgetConfig = config.memory?.budget ?? { ...DEFAULT_MEMORY_BUDGET }

  const updateMemoryBudget = (next: MemoryBudgetConfig | null) => {
    const prevMemory = config.memory ?? DEFAULT_AGENT_MEMORY
    updateConfig({
      memory: {
        ...prevMemory,
        budget: next,
      },
    })
  }

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-auto">
      {/* Active Memory (Phase B1) */}
      <div className="px-6 pt-6 pb-2 shrink-0 w-full">
        <div className="rounded-lg border border-border/60 bg-secondary/20 p-4 space-y-3">
          <div className="flex items-center justify-between">
            <div className="flex flex-col pr-4">
              <label className="text-sm font-semibold">{t("settings.activeMemoryTitle")}</label>
              <p className="text-[11px] text-muted-foreground/70 mt-0.5">
                {t("settings.activeMemoryDesc")}
              </p>
            </div>
            <Switch
              checked={activeMemory.enabled}
              onCheckedChange={(v) => updateActiveMemory({ enabled: v })}
            />
          </div>
          {activeMemory.enabled && (
            <div className="grid grid-cols-2 gap-3 pt-1">
              <label className="flex flex-col gap-1 text-xs">
                <span className="text-muted-foreground">{t("settings.activeMemoryTimeout")}</span>
                <Input
                  type="number"
                  min={200}
                  max={15000}
                  step={100}
                  className="h-8 text-xs"
                  value={activeMemory.timeoutMs}
                  onChange={(e) =>
                    updateActiveMemory({
                      timeoutMs: Math.max(200, Math.min(15000, Number(e.target.value) || 0)),
                    })
                  }
                />
              </label>
              <label className="flex flex-col gap-1 text-xs">
                <span className="text-muted-foreground">{t("settings.activeMemoryCacheTtl")}</span>
                <Input
                  type="number"
                  min={0}
                  max={600}
                  className="h-8 text-xs"
                  value={activeMemory.cacheTtlSecs}
                  onChange={(e) =>
                    updateActiveMemory({
                      cacheTtlSecs: Math.max(0, Math.min(600, Number(e.target.value) || 0)),
                    })
                  }
                />
              </label>
              <label className="flex flex-col gap-1 text-xs">
                <span className="text-muted-foreground">{t("settings.activeMemoryMaxChars")}</span>
                <Input
                  type="number"
                  min={40}
                  max={2000}
                  className="h-8 text-xs"
                  value={activeMemory.maxChars}
                  onChange={(e) =>
                    updateActiveMemory({
                      maxChars: Math.max(40, Math.min(2000, Number(e.target.value) || 0)),
                    })
                  }
                />
              </label>
              <label className="flex flex-col gap-1 text-xs">
                <span className="text-muted-foreground">
                  {t("settings.activeMemoryCandidateLimit")}
                </span>
                <Input
                  type="number"
                  min={1}
                  max={100}
                  className="h-8 text-xs"
                  value={activeMemory.candidateLimit}
                  onChange={(e) =>
                    updateActiveMemory({
                      candidateLimit: Math.max(1, Math.min(100, Number(e.target.value) || 0)),
                    })
                  }
                />
              </label>
            </div>
          )}
        </div>
      </div>

      {/* Memory Budget override (Agent level) */}
      <div className="px-6 pt-2 pb-2 shrink-0 w-full">
        <div className="rounded-lg border border-border/60 bg-secondary/20 p-4 space-y-3">
          <div className="flex items-center justify-between">
            <div className="flex flex-col pr-4">
              <label className="text-sm font-semibold">{t("settings.memoryBudget.title")}</label>
              <p className="text-[11px] text-muted-foreground/70 mt-0.5">
                {t("settings.memoryBudget.agentOverrideDesc")}
              </p>
            </div>
            <div className="flex items-center gap-2">
              <span className="text-xs text-muted-foreground">
                {t("settings.memoryBudget.useGlobalDefault")}
              </span>
              <Switch
                checked={useGlobalBudget}
                onCheckedChange={(v) => updateMemoryBudget(v ? null : { ...DEFAULT_MEMORY_BUDGET })}
              />
            </div>
          </div>
          <MemoryBudgetInputs
            value={budgetValue}
            onChange={(next) => updateMemoryBudget(next)}
            disabled={useGlobalBudget}
          />
        </div>
      </div>

      {/* Core Memory Editor */}
      <div className="px-6 pt-4 pb-4 shrink-0 w-full">
        <div className="flex items-center justify-between mb-1">
          <h3 className="text-sm font-semibold">{t("settings.coreMemory")}</h3>
          {loaded && (
            <Button
              size="sm"
              className="gap-1.5 h-7 text-xs"
              disabled={saving || !hasChanges}
              onClick={handleSave}
              variant={
                saveStatus === "saved"
                  ? "outline"
                  : saveStatus === "failed"
                    ? "destructive"
                    : "default"
              }
            >
              {saving ? (
                <>
                  <Loader2 className="h-3 w-3 animate-spin" />
                  {t("common.saving")}
                </>
              ) : saveStatus === "saved" ? (
                <>
                  <Check className="h-3 w-3" />
                  {t("common.saved")}
                </>
              ) : (
                <>
                  <Save className="h-3 w-3" />
                  {t("common.save")}
                </>
              )}
            </Button>
          )}
        </div>
        <p className="text-xs text-muted-foreground mb-3">{t("settings.coreMemoryAgentDesc")}</p>
        {openclawMode && (
          <div className="rounded-lg border border-green-500/30 bg-green-500/5 px-3 py-2 mb-3">
            <p className="text-xs text-green-600 dark:text-green-400">
              {t("settings.openclawMemoryHint")}
            </p>
          </div>
        )}
        {loaded && (
          <Textarea
            value={content}
            onChange={(e) => setContent(e.target.value)}
            placeholder={t("settings.coreMemoryPlaceholder")}
            className="min-h-[100px] max-h-[200px] text-sm font-mono resize-y"
          />
        )}
      </div>
      {/* Existing Memory Panel */}
      <MemoryPanel agentId={agentId} compact />
    </div>
  )
}
