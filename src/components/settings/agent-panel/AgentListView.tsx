import { useState, useEffect } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  ArrowLeft,
  Bot,
  ChevronRight,
  Download,
  Plus,
} from "lucide-react"
import type { AgentSummary, AgentConfig } from "./types"
import { DEFAULT_PERSONALITY } from "./types"
import OpenClawImportDialog from "./OpenClawImportDialog"

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
  const [error, setError] = useState("")

  const handleCreate = async () => {
    const trimmedId = id.trim().toLowerCase()
    if (!trimmedId) return
    if (!/^[a-z0-9][a-z0-9-]*$/.test(trimmedId)) {
      setError(t("settings.agentNewIdHint"))
      return
    }

    try {
      const config: AgentConfig = {
        name: name.trim() || trimmedId,
        description: null,
        emoji: "🤖",
        avatar: null,
        model: { primary: null, fallbacks: [] },
        personality: { ...DEFAULT_PERSONALITY },
        capabilities: {
          maxToolRounds: 20,
          requireApproval: ["*"],
          sandbox: false,
          skillEnvCheck: true,
          tools: { allow: [], deny: [] },
          skills: { allow: [], deny: [] },
        },
        useCustomPrompt: false,
        openclawMode: false,
        subagents: {
          enabled: true,
          allowedAgents: [],
          deniedAgents: [],
          maxConcurrent: 5,
          defaultTimeoutSecs: 300,
          model: null,
        },
      }
      await getTransport().call("save_agent_config_cmd", { id: trimmedId, config })
      window.dispatchEvent(new Event("agents-changed"))
      onCreated(trimmedId)
    } catch (e) {
      setError(String(e))
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
              className="bg-secondary/40 rounded-lg font-mono"
              value={id}
              onChange={(e) => {
                setId(e.target.value)
                setError("")
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
              className="bg-secondary/40 rounded-lg"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={t("settings.agentNamePlaceholder")}
              onKeyDown={(e) => {
                if (e.key === "Enter") handleCreate()
              }}
            />
          </div>

          {error && <p className="text-xs text-destructive px-1">{error}</p>}

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
  const [creating, setCreating] = useState(false)
  const [importOpen, setImportOpen] = useState(false)

  async function reload() {
    try {
      const list = await getTransport().call<AgentSummary[]>("list_agents")
      setAgents(list)
    } catch (e) {
      logger.error("settings", "AgentPanel::loadAgents", "Failed to load agents", e)
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
      <button
        className="flex items-center gap-2 w-full px-3 py-2.5 rounded-lg text-sm text-primary hover:bg-primary/5 transition-colors"
        onClick={() => setCreating(true)}
      >
        <Plus className="h-4 w-4" />
        <span className="font-medium">{t("settings.agentNew")}</span>
      </button>

      {/* Import from OpenClaw button */}
      <button
        className="flex items-center gap-2 w-full px-3 py-2.5 rounded-lg text-sm text-muted-foreground hover:text-foreground hover:bg-secondary/60 transition-colors mb-3"
        onClick={() => setImportOpen(true)}
      >
        <Download className="h-4 w-4" />
        <span className="font-medium">{t("settings.openclawImportBtn")}</span>
      </button>

      <OpenClawImportDialog
        open={importOpen}
        onOpenChange={setImportOpen}
        onImported={reload}
      />

      <div className="border-t border-border mb-4" />

      {loading ? (
        <div className="flex items-center justify-center py-12">
          <div className="animate-spin h-5 w-5 border-2 border-foreground border-t-transparent rounded-full" />
        </div>
      ) : agents.length === 0 ? (
        <div className="text-center py-12">
          <Bot className="h-10 w-10 text-muted-foreground/30 mx-auto mb-3" />
          <p className="text-sm text-muted-foreground">{t("settings.agentNoAgents")}</p>
          <p className="text-xs text-muted-foreground/70 mt-1">{t("settings.agentNoAgentsHint")}</p>
        </div>
      ) : (
        <div className="space-y-1">
          {agents.map((agent) => (
            <button
              key={agent.id}
              className="flex items-center gap-3 w-full px-3 py-2.5 rounded-lg text-sm transition-colors text-foreground hover:bg-secondary/60 group"
              onClick={() => onEditAgent(agent.id)}
            >
              {/* Avatar / fallback */}
              <div className="w-9 h-9 rounded-full bg-primary/15 flex items-center justify-center text-primary shrink-0 overflow-hidden">
                {agent.avatar ? (
                  <img
                    src={getTransport().resolveAssetUrl(agent.avatar) ?? agent.avatar}
                    className="w-full h-full object-cover"
                    alt=""
                  />
                ) : (
                  <Bot className="h-5 w-5" />
                )}
              </div>

              {/* Name + emoji + description */}
              <div className="flex-1 text-left min-w-0">
                <div className="font-medium truncate flex items-center gap-2">
                  {agent.name}
                  {agent.emoji ? ` ${agent.emoji}` : ""}
                  {agent.id === "default" && (
                    <span className="text-[10px] px-1.5 py-0.5 rounded bg-secondary text-muted-foreground font-medium">
                      {t("settings.agentDefault")}
                    </span>
                  )}
                </div>
                {agent.description && (
                  <div className="text-xs text-muted-foreground truncate">{agent.description}</div>
                )}
              </div>

              <ChevronRight className="h-4 w-4 text-muted-foreground/30 shrink-0 group-hover:text-muted-foreground/60 transition-colors" />
            </button>
          ))}
        </div>
      )}
    </div>
  )
}
