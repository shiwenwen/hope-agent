import { useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
} from "@/components/ui/select"
import { AgentSelectDisplay } from "@/components/common/AgentSelectDisplay"
import type { AgentSummary } from "./types"

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
  const [defaultAgentId, setDefaultAgentId] = useState<string>("default")
  const [loaded, setLoaded] = useState(false)
  const [saving, setSaving] = useState(false)

  useEffect(() => {
    let cancelled = false
    getTransport()
      .call<string | null>("get_default_agent_id")
      .then((currentId) => {
        if (cancelled) return
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

  const selectedAgent = sortedAgents.find((a) => a.id === defaultAgentId)
  const selectedAgentExists = sortedAgents.some((a) => a.id === defaultAgentId)

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
              {defaultAgentId !== "default" && (
                <SelectItem value={defaultAgentId} textValue={defaultAgentId}>
                  <AgentSelectDisplay fallbackName={defaultAgentId} />
                </SelectItem>
              )}
              <SelectItem value="default" textValue="default">
                <AgentSelectDisplay fallbackName="default" />
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
    </section>
  )
}
