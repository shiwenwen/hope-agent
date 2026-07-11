import { useState, useEffect, useCallback } from "react"
import { useTranslation } from "react-i18next"
import { getTransport } from "@/lib/transport-provider"
import { Textarea } from "@/components/ui/textarea"
import { Button } from "@/components/ui/button"
import { AlertCircle, Loader2, Check, Save, ChevronDown, ChevronRight } from "lucide-react"
import { logger } from "@/lib/logger"
import { toast } from "sonner"
import {
  coreMemoryOperationErrorToast,
  coreMemoryOperationForScope,
  type CoreMemoryOperationErrorToast,
} from "./coreMemoryOperationFeedback"

interface CoreMemoryEditorProps {
  scope: "global" | "agent"
  agentId?: string
}

export default function CoreMemoryEditor({ scope, agentId }: CoreMemoryEditorProps) {
  const { t } = useTranslation()
  const [content, setContent] = useState("")
  const [originalContent, setOriginalContent] = useState("")
  const [loaded, setLoaded] = useState(false)
  const [loading, setLoading] = useState(false)
  const [loadError, setLoadError] = useState<CoreMemoryOperationErrorToast | null>(null)
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  const [expanded, setExpanded] = useState(true)

  const loadContent = useCallback(async () => {
    setLoading(true)
    try {
      const md = scope === "global"
        ? await getTransport().call<string | null>("get_global_memory_md")
        : await getTransport().call<string | null>("get_agent_memory_md", { id: agentId })
      const val = md ?? ""
      setContent(val)
      setOriginalContent(val)
      setLoaded(true)
      setLoadError(null)
    } catch (e) {
      logger.error("settings", "CoreMemoryEditor::load", "Failed to load", e)
      setLoadError(coreMemoryOperationErrorToast(
        coreMemoryOperationForScope("load", scope),
        t,
        e,
      ))
    } finally {
      setLoading(false)
    }
  }, [scope, agentId, t])

  useEffect(() => {
    loadContent()
  }, [loadContent])

  useEffect(() => {
    return getTransport().listen("core_memory_updated", (raw) => {
      const payload = raw as { scope: string; agentId?: string }
      if (payload.scope === scope) {
        if (scope === "global" || payload.agentId === agentId) {
          loadContent()
        }
      }
    })
  }, [scope, agentId, loadContent])

  const handleSave = async () => {
    setSaving(true)
    try {
      if (scope === "global") {
        await getTransport().call("save_global_memory_md", { content })
      } else {
        await getTransport().call("save_agent_memory_md", { id: agentId, content })
      }
      setOriginalContent(content)
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } catch (e) {
      logger.error("settings", "CoreMemoryEditor::save", "Failed to save", e)
      setSaveStatus("failed")
      const failureToast = coreMemoryOperationErrorToast(
        coreMemoryOperationForScope("save", scope),
        t,
        e,
      )
      toast.error(
        failureToast.title,
        failureToast.description ? { description: failureToast.description } : undefined,
      )
      setTimeout(() => setSaveStatus("idle"), 2000)
    } finally {
      setSaving(false)
    }
  }

  const hasChanges = content !== originalContent
  const title = scope === "global" ? t("settings.coreMemoryGlobal") : t("settings.coreMemory")
  const desc = scope === "global" ? t("settings.coreMemoryGlobalDesc") : t("settings.coreMemoryAgentDesc")

  return (
    <div className="rounded-lg bg-secondary/30 mb-4 shrink-0">
      <div className="flex items-center justify-between gap-2 pr-3">
        <Button
          variant="ghost"
          onClick={() => setExpanded(!expanded)}
          className="h-auto flex-1 justify-start gap-1.5 rounded-none rounded-l-lg px-3 py-2 font-normal hover:bg-transparent"
        >
          {expanded ? <ChevronDown className="h-3.5 w-3.5 text-muted-foreground" /> : <ChevronRight className="h-3.5 w-3.5 text-muted-foreground" />}
          <span className="text-sm font-medium">{title}</span>
          {loading && <Loader2 className="h-3 w-3 animate-spin text-muted-foreground" />}
          {originalContent.trim() && !expanded && (
            <span className="text-[10px] text-muted-foreground ml-1">
              ({originalContent.trim().split("\n").length} {t("settings.coreMemoryLines")})
            </span>
          )}
        </Button>
        {loaded && hasChanges && expanded && (
          <Button
            size="sm"
            className="gap-1.5 h-6 text-xs shrink-0"
            disabled={saving}
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
      {expanded && (
        <div className="px-3 pb-3">
          <p className="text-xs text-muted-foreground mb-2">{desc}</p>
          {loadError && (
            <div className="mb-2 rounded-md border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs">
              <div className="flex items-center gap-1.5 font-medium text-foreground">
                <AlertCircle className="h-3.5 w-3.5 text-amber-500" />
                {loadError.title}
              </div>
              {loadError.description && (
                <div className="mt-1 break-all text-muted-foreground">{loadError.description}</div>
              )}
              <button
                type="button"
                className="mt-2 font-medium text-foreground underline underline-offset-2"
                onClick={() => void loadContent()}
              >
                {t("common.retry", "Retry")}
              </button>
            </div>
          )}
          {loaded && (
            <Textarea
              value={content}
              onChange={(e) => setContent(e.target.value)}
              placeholder={t("settings.coreMemoryPlaceholder")}
              className="min-h-[80px] max-h-[200px] text-sm font-mono resize-y"
            />
          )}
        </div>
      )}
    </div>
  )
}
