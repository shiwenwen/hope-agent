import { useCallback, useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { Button } from "@/components/ui/button"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
} from "@/components/ui/select"
import { AgentSelectDisplay } from "@/components/common/AgentSelectDisplay"
import { DEFAULT_AGENT_ID } from "@/types/tools"
import type { AgentSummary } from "./types"
import { AlertCircle, RefreshCw } from "lucide-react"
import {
  agentOperationErrorToast,
  type AgentLoadOperationErrorToast,
} from "./agentLoadOperationFeedback"

interface DefaultAgentSectionProps {
  agents: AgentSummary[]
  loading?: boolean
}

/**
 * Global default agent selector. Used as the fallback when neither the
 * caller, the project, nor the IM channel-account specifies an agent.
 *
 * See `AppConfig.default_agent_id` and `crate::agent::resolver` in the
 * backend for the precedence chain.
 */
export default function DefaultAgentSection({
  agents,
  loading = false,
}: DefaultAgentSectionProps) {
  const { t } = useTranslation()
  const [defaultAgentId, setDefaultAgentId] = useState<string>(DEFAULT_AGENT_ID)
  const [loaded, setLoaded] = useState(false)
  const [saving, setSaving] = useState(false)
  const [loadError, setLoadError] = useState<AgentLoadOperationErrorToast | null>(null)
  const [saveError, setSaveError] = useState<AgentLoadOperationErrorToast | null>(null)

  const loadDefaultAgent = useCallback(
    async (options: { cancelled?: () => boolean } = {}) => {
      setLoaded(false)
      setLoadError(null)
      try {
        const currentId = await getTransport().call<string | null>("get_default_agent_id")
        if (options.cancelled?.()) return
        const id =
          typeof currentId === "string" && currentId.trim().length > 0
            ? currentId
            : DEFAULT_AGENT_ID
        setDefaultAgentId(id)
        setSaveError(null)
        setLoaded(true)
      } catch (e) {
        if (options.cancelled?.()) return
        logger.error(
          "settings",
          "DefaultAgentSection::load",
          "Failed to load default agent",
          e,
        )
        setLoadError(agentOperationErrorToast("load", t, e))
      }
    },
    [t],
  )

  useEffect(() => {
    let cancelled = false
    void loadDefaultAgent({ cancelled: () => cancelled })
    return () => {
      cancelled = true
    }
  }, [loadDefaultAgent])

  const sortedAgents = useMemo(() => {
    return [...agents].sort((a, b) => a.name.localeCompare(b.name))
  }, [agents])

  const selectedAgent = sortedAgents.find((a) => a.id === defaultAgentId)
  const selectedAgentExists = sortedAgents.some((a) => a.id === defaultAgentId)

  async function handleChange(nextId: string) {
    const previous = defaultAgentId
    setDefaultAgentId(nextId)
    setSaving(true)
    setSaveError(null)
    try {
      await getTransport().call("set_default_agent_id", { agentId: nextId })
    } catch (e) {
      logger.error("settings", "DefaultAgentSection::save", "Failed to save default agent", e)
      setDefaultAgentId(previous)
      setSaveError(agentOperationErrorToast("save", t, e))
    } finally {
      setSaving(false)
    }
  }

  return (
    <section className="mb-4 space-y-2 rounded-lg px-3 py-3 transition-colors hover:bg-secondary/40">
      <div className="space-y-0.5">
        <div className="text-sm font-medium">{t("settings.defaultAgentLabel")}</div>
        <div className="text-xs text-muted-foreground">{t("settings.defaultAgentDesc")}</div>
      </div>
      <Select
        value={defaultAgentId}
        disabled={!loaded || loading || saving}
        onValueChange={(v) => void handleChange(v)}
      >
        <SelectTrigger className="h-9 w-full max-w-sm overflow-hidden text-sm">
          <div className="flex min-w-0 flex-1 items-center overflow-hidden">
            <AgentSelectDisplay agent={selectedAgent} fallbackName={defaultAgentId} />
          </div>
        </SelectTrigger>
        <SelectContent>
          {sortedAgents.length === 0 ? (
            <>
              {defaultAgentId !== DEFAULT_AGENT_ID && (
                <SelectItem value={defaultAgentId} textValue={defaultAgentId}>
                  <AgentSelectDisplay fallbackName={defaultAgentId} />
                </SelectItem>
              )}
              <SelectItem value={DEFAULT_AGENT_ID} textValue={DEFAULT_AGENT_ID}>
                <AgentSelectDisplay fallbackName={DEFAULT_AGENT_ID} />
              </SelectItem>
            </>
          ) : (
            <>
              {!selectedAgentExists && (
                <SelectItem value={defaultAgentId} textValue={defaultAgentId}>
                  <AgentSelectDisplay fallbackName={defaultAgentId} />
                </SelectItem>
              )}
              {sortedAgents.map((a) => (
                <SelectItem key={a.id} value={a.id} textValue={a.name}>
                  <AgentSelectDisplay agent={a} />
                </SelectItem>
              ))}
            </>
          )}
        </SelectContent>
      </Select>
      {loadError && (
        <div className="rounded-md border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs">
          <div className="flex items-center gap-1.5 font-medium text-foreground">
            <AlertCircle className="h-3.5 w-3.5 text-amber-500" />
            {loadError.title}
          </div>
          {loadError.description && (
            <div className="mt-1 break-all text-muted-foreground">{loadError.description}</div>
          )}
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="mt-2 h-7 gap-1.5 px-2 text-xs"
            onClick={() => void loadDefaultAgent()}
          >
            <RefreshCw className="h-3.5 w-3.5" />
            {t("common.retry")}
          </Button>
        </div>
      )}
      {saveError && (
        <div className="rounded-md border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs">
          <div className="flex items-center gap-1.5 font-medium text-foreground">
            <AlertCircle className="h-3.5 w-3.5 text-amber-500" />
            {saveError.title}
          </div>
          {saveError.description && (
            <div className="mt-1 break-all text-muted-foreground">{saveError.description}</div>
          )}
        </div>
      )}
    </section>
  )
}
