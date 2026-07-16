import { useState, useEffect } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { AlertCircle, ArrowLeft, Bot, ChevronRight, Download, Plus, RefreshCw } from "lucide-react"
import type { AgentSummary, AgentConfig } from "./types"
import { DEFAULT_PERSONALITY } from "./types"
import { isMainAgent } from "@/types/tools"
import OpenClawImportDialog from "./OpenClawImportDialog"
import DefaultAgentSection from "./DefaultAgentSection"
import { AgentAvatarBadge } from "@/components/common/AgentSelectDisplay"
import {
  agentLoadOperationErrorToast,
  agentOperationErrorToast,
  type AgentLoadOperationErrorToast,
} from "./agentLoadOperationFeedback"

// ── Agent Create View ───────────────────────────────────────────

function AgentCreateView({
  onBack,
  onCreated,
}: {
  onBack: () => void
  onCreated: (id: string) => void
}) {
  const { t } = useTranslation()
  const [id, setId] = useState("")
  const [name, setName] = useState("")
  const [error, setError] = useState<AgentLoadOperationErrorToast | null>(null)

  const handleCreate = async () => {
    const trimmedId = id.trim().toLowerCase()
    if (!trimmedId) return
    if (!/^[a-z0-9][a-z0-9-]*$/.test(trimmedId)) {
      setError({ title: t("settings.agentNewIdHint") })
      return
    }

    try {
      const config: AgentConfig = {
        enabled: true,
        name: name.trim() || trimmedId,
        description: null,
        emoji: null,
        avatar: null,
        model: { primary: null, fallbacks: [] },
        personality: { ...DEFAULT_PERSONALITY },
        capabilities: {
          maxToolRounds: 0,
          sandbox: false,
          skillEnvCheck: true,
          tools: { allow: [], deny: [] },
          skills: { allow: [], deny: [] },
        },
        openclawMode: false,
        subagents: {
          allowedAgents: [],
          deniedAgents: [],
          maxConcurrent: 8,
          defaultTimeoutSecs: 0,
          model: null,
        },
      }
      await getTransport().call("save_agent_config_cmd", { id: trimmedId, config, create: true })
      window.dispatchEvent(new Event("agents-changed"))
      onCreated(trimmedId)
    } catch (e) {
      setError(agentOperationErrorToast("save", t, e))
    }
  }

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-y-auto p-6">
      <div className="w-full">
        <Button
          variant="ghost"
          size="sm"
          onClick={onBack}
          className="gap-1.5 text-muted-foreground hover:text-foreground mb-4"
        >
          <ArrowLeft className="h-4 w-4" />
          <span>{t("settings.agents")}</span>
        </Button>

        <h2 className="text-lg font-semibold text-foreground mb-5">{t("settings.agentNew")}</h2>

        <div className="space-y-4">
          <div>
            <div className="text-xs font-medium text-muted-foreground mb-2 px-1">
              {t("settings.agentNewId")}
            </div>
            <Input
              className="font-mono"
              value={id}
              onChange={(e) => {
                setId(e.target.value)
                setError(null)
              }}
              placeholder={t("settings.agentNewIdPlaceholder")}
              autoFocus
            />
            <p className="text-[11px] text-muted-foreground/60 mt-1 px-1">
              {t("settings.agentNewIdHint")}
            </p>
          </div>

          <div>
            <div className="text-xs font-medium text-muted-foreground mb-2 px-1">
              {t("settings.agentName")}
            </div>
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={t("settings.agentNamePlaceholder")}
              onKeyDown={(e) => {
                if (e.key === "Enter") handleCreate()
              }}
            />
          </div>

          {error && (
            <div className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs">
              <div className="font-medium text-destructive">{error.title}</div>
              {error.description && (
                <div className="mt-1 break-all text-muted-foreground">{error.description}</div>
              )}
            </div>
          )}

          <Button onClick={handleCreate} disabled={!id.trim()}>
            {t("common.add")}
          </Button>
        </div>
      </div>
    </div>
  )
}

// ── Agent List View ───────────────────────────────────────────

export default function AgentListView({ onEditAgent }: { onEditAgent: (id: string) => void }) {
  const { t } = useTranslation()
  const [agents, setAgents] = useState<AgentSummary[]>([])
  const [loading, setLoading] = useState(true)
  const [listError, setListError] = useState<AgentLoadOperationErrorToast | null>(null)
  const [creating, setCreating] = useState(false)
  const [importOpen, setImportOpen] = useState(false)

  async function reload() {
    setLoading(true)
    try {
      const list = await getTransport().call<AgentSummary[]>("list_all_agents")
      setAgents(list)
      setListError(null)
    } catch (e) {
      logger.error("settings", "AgentPanel::loadAgents", "Failed to load agents", e)
      setListError(agentLoadOperationErrorToast(t, e))
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    reload()
  }, [])

  if (creating) {
    return (
      <AgentCreateView
        onBack={() => setCreating(false)}
        onCreated={(id) => {
          setCreating(false)
          onEditAgent(id)
        }}
      />
    )
  }

  return (
    <div className="flex-1 min-h-0 overflow-y-auto p-6">
      <h2 className="text-lg font-semibold text-foreground mb-1">{t("settings.agents")}</h2>
      <p className="text-xs text-muted-foreground mb-4">{t("settings.agentsDesc")}</p>

      {/* New Agent button */}
      <Button
        variant="ghost"
        className="h-auto w-full justify-start gap-2 rounded-lg px-3 py-2.5 text-sm font-medium text-primary hover:bg-primary/5 hover:text-primary"
        onClick={() => setCreating(true)}
      >
        <Plus className="h-4 w-4" />
        <span>{t("settings.agentNew")}</span>
      </Button>

      <OpenClawImportDialog open={importOpen} onOpenChange={setImportOpen} onImported={reload} />

      <div className="border-t border-border mb-4 mt-3" />

      {loading ? (
        <div className="flex items-center justify-center py-12">
          <div className="animate-spin h-5 w-5 border-2 border-foreground border-t-transparent rounded-full" />
        </div>
      ) : agents.length === 0 ? (
        listError ? (
          <div className="rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-3 text-xs">
            <div className="flex items-center gap-1.5 font-medium text-foreground">
              <AlertCircle className="h-3.5 w-3.5 text-amber-500" />
              {listError.title}
            </div>
            {listError.description && (
              <div className="mt-1 break-all text-muted-foreground">{listError.description}</div>
            )}
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="mt-2 h-7 gap-1.5 px-2 text-xs"
              onClick={() => void reload()}
            >
              <RefreshCw className="h-3.5 w-3.5" />
              {t("common.retry")}
            </Button>
          </div>
        ) : (
          <div className="text-center py-12">
            <Bot className="h-10 w-10 text-muted-foreground/30 mx-auto mb-3" />
            <p className="text-sm text-muted-foreground">{t("settings.agentNoAgents")}</p>
            <p className="text-xs text-muted-foreground/70 mt-1">
              {t("settings.agentNoAgentsHint")}
            </p>
          </div>
        )
      ) : (
        <div className="space-y-1">
          {listError && (
            <div className="mb-3 rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs">
              <div className="flex items-center gap-1.5 font-medium text-foreground">
                <AlertCircle className="h-3.5 w-3.5 text-amber-500" />
                {listError.title}
              </div>
              {listError.description && (
                <div className="mt-1 break-all text-muted-foreground">{listError.description}</div>
              )}
            </div>
          )}
          {agents.map((agent) => (
            <Button
              key={agent.id}
              variant="ghost"
              className={cn(
                "group h-auto w-full justify-start gap-3 rounded-lg px-3 py-2.5 text-sm font-normal text-foreground hover:bg-secondary/60",
                agent.enabled === false && "opacity-60",
              )}
              onClick={() => onEditAgent(agent.id)}
            >
              <AgentAvatarBadge agent={agent} size="lg" />

              {/* Name + description */}
              <div className="flex-1 text-left min-w-0">
                <div className="font-medium truncate flex items-center gap-2">
                  {agent.name}
                  {isMainAgent(agent.id) && (
                    <span className="text-[10px] px-1.5 py-0.5 rounded bg-secondary text-muted-foreground font-medium">
                      {t("settings.agentDefault")}
                    </span>
                  )}
                  {agent.enabled === false && (
                    <span className="rounded bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
                      {t("agentLifecycle.disabled")}
                    </span>
                  )}
                </div>
                {agent.description && (
                  <div className="text-xs text-muted-foreground truncate">{agent.description}</div>
                )}
              </div>

              <ChevronRight className="h-4 w-4 text-muted-foreground/30 shrink-0 group-hover:text-muted-foreground/60 transition-colors" />
            </Button>
          ))}
        </div>
      )}

      <div className="mt-6 border-t border-border pt-4">
        <DefaultAgentSection agents={agents} loading={loading} />
      </div>

      <div className="mt-6 border-t border-border/60 pt-3">
        <Button
          variant="ghost"
          size="sm"
          className="h-8 gap-1.5 px-2 text-xs font-normal text-muted-foreground hover:text-foreground"
          onClick={() => setImportOpen(true)}
        >
          <Download className="h-3.5 w-3.5" />
          <span>{t("settings.openclawImportBtn")}</span>
        </Button>
      </div>
    </div>
  )
}
