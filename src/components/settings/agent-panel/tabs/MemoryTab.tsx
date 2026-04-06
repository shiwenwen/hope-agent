import { useState, useEffect, useCallback } from "react"
import { useTranslation } from "react-i18next"
import { getTransport } from "@/lib/transport-provider"
import { Textarea } from "@/components/ui/textarea"
import { Button } from "@/components/ui/button"
import { Loader2, Check, Save } from "lucide-react"
import MemoryPanel from "@/components/settings/MemoryPanel"
import { logger } from "@/lib/logger"

interface MemoryTabProps {
  agentId: string
  openclawMode?: boolean
}

export default function MemoryTab({ agentId, openclawMode }: MemoryTabProps) {
  const { t } = useTranslation()
  const [content, setContent] = useState("")
  const [originalContent, setOriginalContent] = useState("")
  const [loaded, setLoaded] = useState(false)
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")

  const loadContent = useCallback(async () => {
    try {
      const md = await getTransport().call<string | null>("get_agent_memory_md", { id: agentId })
      const val = md ?? ""
      setContent(val)
      setOriginalContent(val)
      setLoaded(true)
    } catch (e) {
      logger.error("settings", "MemoryTab::loadCoreMemory", "Failed to load", e)
    }
  }, [agentId])

  useEffect(() => {
    loadContent()
  }, [loadContent])

  // Listen for updates from the agent tool
  useEffect(() => {
    const unlisten = getTransport().listen("core_memory_updated", (raw) => {
      const payload = raw as { agentId: string; scope: string }
      if (payload.scope === "agent" && payload.agentId === agentId) {
        loadContent()
      }
    })
    return unlisten
  }, [agentId, loadContent])

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

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-auto">
      {/* Core Memory Editor */}
      <div className="px-6 pt-6 pb-4 shrink-0 max-w-4xl w-full">
        <div className="flex items-center justify-between mb-1">
          <h3 className="text-sm font-semibold">{t("settings.coreMemory")}</h3>
          {loaded && (
            <Button
              size="sm"
              className="gap-1.5 h-7 text-xs"
              disabled={saving || !hasChanges}
              onClick={handleSave}
              variant={saveStatus === "saved" ? "outline" : saveStatus === "failed" ? "destructive" : "default"}
            >
              {saving ? (
                <><Loader2 className="h-3 w-3 animate-spin" />{t("common.saving")}</>
              ) : saveStatus === "saved" ? (
                <><Check className="h-3 w-3" />{t("common.saved")}</>
              ) : (
                <><Save className="h-3 w-3" />{t("common.save")}</>
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
