import { useState, useEffect } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { Switch } from "@/components/ui/switch"
import { DeferredNumberInput } from "@/components/ui/deferred-number-input"
import { Button } from "@/components/ui/button"
import { Check, Bot } from "lucide-react"
import type { AgentConfig, AgentSummary } from "./types"

interface SubagentPanelProps {
  config: AgentConfig["subagents"]
  enabled: boolean
  currentAgentId: string
  onChange: (config: AgentConfig["subagents"]) => void
  onEnabledChange: (enabled: boolean) => void
}

export default function SubagentPanel({
  config,
  enabled,
  currentAgentId,
  onChange,
  onEnabledChange,
}: SubagentPanelProps) {
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
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0 space-y-1">
          <span className="text-sm">{t("settings.subagentEnabled")}</span>
          <p className="text-xs leading-relaxed text-muted-foreground">
            {t("settings.subagentEnabledDesc")}
          </p>
        </div>
        <Switch checked={enabled} onCheckedChange={onEnabledChange} />
      </div>

      {enabled && (
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
                    <Button
                      key={agent.id}
                      variant="outline"
                      className="h-auto w-full justify-start gap-2 rounded-lg px-3 py-2 text-left font-normal hover:bg-secondary/60"
                      onClick={() => toggleAgent(agent.id)}
                    >
                      <div
                        className={`flex h-4 w-4 shrink-0 items-center justify-center rounded border ${enabled ? "border-transparent bg-primary" : "border-muted-foreground/40"}`}
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
                    </Button>
                  )
                })}
              </div>
            </div>
          )}

          {/* Max spawn depth */}
          <div className="space-y-1">
            <div className="flex items-start justify-between gap-4">
              <div className="min-w-0 space-y-1">
                <span className="text-sm">{t("settings.subagentMaxDepth")}</span>
                <p className="text-xs leading-relaxed text-muted-foreground">
                  {t("settings.subagentMaxDepthDesc")}
                </p>
              </div>
              <DeferredNumberInput
                value={config.maxSpawnDepth ?? 3}
                onValueCommit={(value) =>
                  onChange({
                    ...config,
                    maxSpawnDepth: value,
                  })
                }
                className="w-20 shrink-0 text-sm"
                min={1}
                max={5}
              />
            </div>
          </div>

          {/* Max concurrent */}
          <div className="space-y-1">
            <div className="flex items-start justify-between gap-4">
              <div className="min-w-0 space-y-1">
                <span className="text-sm">{t("settings.subagentMaxConcurrent")}</span>
                <p className="text-xs leading-relaxed text-muted-foreground">
                  {t("settings.subagentMaxConcurrentDesc")}
                </p>
              </div>
              <DeferredNumberInput
                value={config.maxConcurrent}
                onValueCommit={(value) =>
                  onChange({
                    ...config,
                    maxConcurrent: value,
                  })
                }
                className="w-20 shrink-0 text-sm"
                min={1}
                max={50}
              />
            </div>
          </div>

          {/* Max batch size */}
          <div className="space-y-1">
            <div className="flex items-start justify-between gap-4">
              <div className="min-w-0 space-y-1">
                <span className="text-sm">{t("settings.subagentMaxBatchSize")}</span>
                <p className="text-xs leading-relaxed text-muted-foreground">
                  {t("settings.subagentMaxBatchSizeDesc")}
                </p>
              </div>
              <DeferredNumberInput
                value={config.maxBatchSize ?? 10}
                onValueCommit={(value) =>
                  onChange({
                    ...config,
                    maxBatchSize: value,
                  })
                }
                className="w-20 shrink-0 text-sm"
                min={1}
                max={50}
              />
            </div>
          </div>

          {/* Default timeout */}
          <div className="space-y-1">
            <div className="flex items-start justify-between gap-4">
              <div className="min-w-0 space-y-1">
                <span className="text-sm">{t("settings.subagentTimeout")}</span>
                <p className="text-xs leading-relaxed text-muted-foreground">
                  {t("settings.subagentTimeoutDesc")}
                </p>
              </div>
              <div className="flex shrink-0 items-center gap-2">
                <DeferredNumberInput
                  value={config.defaultTimeoutSecs}
                  onValueCommit={(value) =>
                    onChange({
                      ...config,
                      defaultTimeoutSecs: value,
                    })
                  }
                  className="w-24 text-sm"
                  min={0}
                  max={1800}
                />
                <span className="text-xs text-muted-foreground">
                  {t("settings.subagentTimeoutUnit")}
                </span>
              </div>
            </div>
          </div>

          {/* Announce timeout */}
          <div className="space-y-1">
            <div className="flex items-start justify-between gap-4">
              <div className="min-w-0 space-y-1">
                <span className="text-sm">{t("settings.subagentAnnounceTimeout")}</span>
                <p className="text-xs leading-relaxed text-muted-foreground">
                  {t("settings.subagentAnnounceTimeoutDesc")}
                </p>
              </div>
              <div className="flex shrink-0 items-center gap-2">
                <DeferredNumberInput
                  value={config.announceTimeoutSecs ?? 120}
                  onValueCommit={(value) =>
                    onChange({
                      ...config,
                      announceTimeoutSecs: value,
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
          </div>
        </>
      )}
    </div>
  )
}
