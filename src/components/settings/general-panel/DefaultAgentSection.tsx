import { useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import type { AgentSummaryForSidebar } from "@/types/chat"

/**
 * Global default agent selector. Used as the fallback when neither the
 * caller, the project, nor the IM channel-account specifies an agent.
 *
 * See `AppConfig.default_agent_id` and `crate::agent::resolver` in the
 * backend for the precedence chain.
 */
export default function DefaultAgentSection() {
  const { t } = useTranslation()
  const [agents, setAgents] = useState<AgentSummaryForSidebar[]>([])
  const [defaultAgentId, setDefaultAgentId] = useState<string>("default")
  const [loaded, setLoaded] = useState(false)
  const [saving, setSaving] = useState(false)

  useEffect(() => {
    let cancelled = false
    Promise.all([
      getTransport().call<AgentSummaryForSidebar[]>("list_agents"),
      getTransport()
        .call<string | null>("get_default_agent_id")
        .catch(() => null),
    ])
      .then(([allAgents, currentId]) => {
        if (cancelled) return
        setAgents(allAgents ?? [])
        const id =
          typeof currentId === "string" && currentId.trim().length > 0 ? currentId : "default"
        setDefaultAgentId(id)
        setLoaded(true)
      })
      .catch((e) => {
        logger.error(
          "settings",
          "DefaultAgentSection::load",
          "Failed to load default agent",
          e,
        )
        setLoaded(true)
      })
    return () => {
      cancelled = true
    }
  }, [])

  const sortedAgents = useMemo(() => {
    return [...agents].sort((a, b) => a.name.localeCompare(b.name))
  }, [agents])

  async function handleChange(nextId: string) {
    const previous = defaultAgentId
    setDefaultAgentId(nextId)
    setSaving(true)
    try {
      await getTransport().call("set_default_agent_id", { agentId: nextId })
    } catch (e) {
      logger.error("settings", "DefaultAgentSection::save", "Failed to save default agent", e)
      setDefaultAgentId(previous)
    } finally {
      setSaving(false)
    }
  }

  return (
    <div>
      <h3 className="text-sm font-semibold text-foreground mb-1">
        {t("settings.defaultAgentTitle")}
      </h3>
      {loaded && (
        <div className="px-3 py-3 rounded-lg hover:bg-secondary/40 transition-colors space-y-2">
          <div className="space-y-0.5">
            <div className="text-sm font-medium">{t("settings.defaultAgentLabel")}</div>
            <div className="text-xs text-muted-foreground">
              {t("settings.defaultAgentDesc")}
            </div>
          </div>
          <Select
            value={defaultAgentId}
            disabled={saving}
            onValueChange={(v) => void handleChange(v)}
          >
            <SelectTrigger className="w-full max-w-sm h-8 text-sm">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {sortedAgents.length === 0 ? (
                <SelectItem value="default">default</SelectItem>
              ) : (
                sortedAgents.map((a) => (
                  <SelectItem key={a.id} value={a.id}>
                    {a.emoji ? `${a.emoji} ` : ""}
                    {a.name} ({a.id})
                  </SelectItem>
                ))
              )}
            </SelectContent>
          </Select>
        </div>
      )}
    </div>
  )
}
