import { useState, useEffect } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { Switch } from "@/components/ui/switch"
import { Input } from "@/components/ui/input"
import { Check, Bot } from "lucide-react"
import type { AgentConfig, AgentSummary } from "./types"

interface SubagentPanelProps {
  config: AgentConfig["subagents"]
  currentAgentId: string
  onChange: (config: AgentConfig["subagents"]) => void
}

export default function SubagentPanel({ config, currentAgentId, onChange }: SubagentPanelProps) {
  const { t } = useTranslation()
  const [agents, setAgents] = useState<AgentSummary[]>([])

  useEffect(() => {
    getTransport().call<AgentSummary[]>("list_agents")
      .then((list) => {
        // Exclude self from the list
        setAgents(list.filter((a) => a.id !== currentAgentId))
      })
      .catch(() => {})
  }, [currentAgentId])

  const isAgentEnabled = (agentId: string) => {
    // If allowedAgents is empty, all are allowed (unless denied)
    if (config.deniedAgents.includes(agentId)) return false
    if (config.allowedAgents.length === 0) return true
    return config.allowedAgents.includes(agentId)
  }

  const toggleAgent = (agentId: string) => {
    if (isAgentEnabled(agentId)) {
      // Disable: add to denied list
      onChange({
        ...config,
        deniedAgents: [...config.deniedAgents.filter((id) => id !== agentId), agentId],
        allowedAgents: config.allowedAgents.filter((id) => id !== agentId),
      })
    } else {
      // Enable: remove from denied, add to allowed if allowedAgents is non-empty
      const newDenied = config.deniedAgents.filter((id) => id !== agentId)
      const newAllowed =
        config.allowedAgents.length > 0
          ? [...config.allowedAgents.filter((id) => id !== agentId), agentId]
          : []
      onChange({ ...config, deniedAgents: newDenied, allowedAgents: newAllowed })
    }
  }

  return (
    <div className="space-y-4">
      <h3 className="text-sm font-medium">{t("settings.subagentTitle")}</h3>

      {/* Enable toggle */}
      <div className="flex items-center justify-between">
        <span className="text-sm">{t("settings.subagentEnabled")}</span>
        <Switch
          checked={config.enabled}
          onCheckedChange={(checked) => onChange({ ...config, enabled: checked })}
        />
      </div>

      {config.enabled && (
        <>
          {/* Enabled sub-agents selection */}
          {agents.length > 0 && (
            <div className="space-y-2">
              <span className="text-sm">{t("settings.subagentAllowedAgents")}</span>
              <p className="text-xs text-muted-foreground">
                {t("settings.subagentAllowedAgentsHint")}
              </p>
              <div className="space-y-1">
                {agents.map((agent) => {
                  const enabled = isAgentEnabled(agent.id)
                  return (
                    <button
                      key={agent.id}
                      className="flex items-center gap-2 w-full px-3 py-2 rounded-lg border border-border hover:bg-secondary/60 transition-colors text-left"
                      onClick={() => toggleAgent(agent.id)}
                    >
                      <div
                        className={`flex items-center justify-center h-4 w-4 rounded border shrink-0 ${enabled ? "bg-primary border-primary" : "border-muted-foreground/40"}`}
                      >
                        {enabled && <Check className="h-3 w-3 text-primary-foreground" />}
                      </div>
                      <div className="flex items-center justify-center h-6 w-6 rounded-full bg-secondary overflow-hidden shrink-0">
                        {agent.avatar ? (
                          <img
                            src={getTransport().resolveAssetUrl(agent.avatar) ?? agent.avatar}
                            className="w-full h-full object-cover"
                            alt=""
                          />
                        ) : agent.emoji ? (
                          <span className="text-sm">{agent.emoji}</span>
                        ) : (
                          <Bot className="h-3.5 w-3.5 text-muted-foreground" />
                        )}
                      </div>
                      <span className="text-sm font-medium flex-1">{agent.name}</span>
                      {agent.description && (
                        <span className="text-xs text-muted-foreground truncate max-w-[200px]">
                          {agent.description}
                        </span>
                      )}
                    </button>
                  )
                })}
              </div>
            </div>
          )}

          {/* Max spawn depth */}
          <div className="space-y-1">
            <div className="flex items-center justify-between">
              <span className="text-sm">{t("settings.subagentMaxDepth")}</span>
              <Input
                type="number"
                value={config.maxSpawnDepth ?? 3}
                onChange={(e) =>
                  onChange({
                    ...config,
                    maxSpawnDepth: Math.max(1, Math.min(5, Number(e.target.value) || 3)),
                  })
                }
                className="w-20 text-sm"
                min={1}
                max={5}
              />
            </div>
          </div>

          {/* Max concurrent */}
          <div className="space-y-1">
            <div className="flex items-center justify-between">
              <span className="text-sm">{t("settings.subagentMaxConcurrent")}</span>
              <Input
                type="number"
                value={config.maxConcurrent}
                onChange={(e) =>
                  onChange({
                    ...config,
                    maxConcurrent: Math.max(1, Math.min(10, Number(e.target.value) || 5)),
                  })
                }
                className="w-20 text-sm"
                min={1}
                max={10}
              />
            </div>
          </div>

          {/* Max batch size */}
          <div className="space-y-1">
            <div className="flex items-center justify-between">
              <span className="text-sm">{t("settings.subagentMaxBatchSize")}</span>
              <Input
                type="number"
                value={config.maxBatchSize ?? 10}
                onChange={(e) =>
                  onChange({
                    ...config,
                    maxBatchSize: Math.max(1, Math.min(50, Number(e.target.value) || 10)),
                  })
                }
                className="w-20 text-sm"
                min={1}
                max={50}
              />
            </div>
          </div>

          {/* Default timeout */}
          <div className="space-y-1">
            <span className="text-sm">{t("settings.subagentTimeout")}</span>
            <div className="flex items-center gap-2">
              <Input
                type="number"
                value={config.defaultTimeoutSecs}
                onChange={(e) =>
                  onChange({
                    ...config,
                    defaultTimeoutSecs: Math.max(30, Math.min(1800, Number(e.target.value) || 300)),
                  })
                }
                className="w-24 text-sm"
                min={30}
                max={1800}
              />
              <span className="text-xs text-muted-foreground">
                {t("settings.subagentTimeoutUnit")}
              </span>
            </div>
          </div>

          {/* Announce timeout */}
          <div className="space-y-1">
            <span className="text-sm">{t("settings.subagentAnnounceTimeout")}</span>
            <div className="flex items-center gap-2">
              <Input
                type="number"
                value={config.announceTimeoutSecs ?? 120}
                onChange={(e) =>
                  onChange({
                    ...config,
                    announceTimeoutSecs: Math.max(10, Math.min(600, Number(e.target.value) || 120)),
                  })
                }
                className="w-24 text-sm"
                min={10}
                max={600}
              />
              <span className="text-xs text-muted-foreground">
                {t("settings.subagentTimeoutUnit")}
              </span>
            </div>
          </div>
        </>
      )}
    </div>
  )
}
