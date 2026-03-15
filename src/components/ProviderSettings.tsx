import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import type { ModelConfig } from "@/components/ProviderSetup"
import ProviderIcon from "@/components/ProviderIcon"
import {
  Loader2,
  MoreVertical,
  Pencil,
  Plus,
  Power,
  PowerOff,
  RefreshCw,
  Trash2,
} from "lucide-react"

// ── Types (shared with ProviderSetup) ─────────────────────────────

type ApiType = "anthropic" | "openai-chat" | "openai-responses" | "codex"

export interface ProviderConfig {
  id: string
  name: string
  apiType: ApiType
  baseUrl: string
  apiKey: string
  models: ModelConfig[]
  enabled: boolean
  userAgent: string
}

// ── Helpers ───────────────────────────────────────────────────────

function apiTypeLabel(type: ApiType) {
  switch (type) {
    case "anthropic":
      return "Anthropic"
    case "openai-chat":
      return "OpenAI Chat"
    case "openai-responses":
      return "OpenAI Responses"
    case "codex":
      return "Codex OAuth"
  }
}

// ── Main Component ────────────────────────────────────────────────

export default function ProviderSettings({
  onAddProvider,
  onEditProvider,
  onCodexReauth,
}: {
  onAddProvider: () => void
  onEditProvider: (provider: ProviderConfig) => void
  onCodexReauth?: () => void
}) {
  const { t } = useTranslation()
  const [providers, setProviders] = useState<ProviderConfig[]>([])
  const [loading, setLoading] = useState(true)
  const [menuId, setMenuId] = useState<string | null>(null)

  useEffect(() => {
    loadProviders()
  }, [])

  async function loadProviders() {
    setLoading(true)
    try {
      const list = await invoke<ProviderConfig[]>("get_providers")
      setProviders(list)
    } catch (e) {
      console.error("Failed to load providers:", e)
    } finally {
      setLoading(false)
    }
  }

  async function deleteProvider(id: string) {
    if (!confirm(t("provider.confirmDelete"))) return
    try {
      await invoke("delete_provider", { providerId: id })
      await loadProviders()
    } catch (e) {
      console.error("Failed to delete provider:", e)
    }
    setMenuId(null)
  }

  async function toggleProvider(provider: ProviderConfig) {
    try {
      await invoke("update_provider", {
        config: { ...provider, enabled: !provider.enabled },
      })
      await loadProviders()
    } catch (e) {
      console.error("Failed to toggle provider:", e)
    }
    setMenuId(null)
  }

  return (
    <div className="flex flex-col h-full">
      {/* Add Provider Button */}
      <div className="flex items-center justify-between px-5 pt-5 pb-2">
        <h2 className="text-lg font-semibold text-foreground">
          {t("provider.title")}
        </h2>
        <Button variant="secondary" size="sm" onClick={onAddProvider}>
          <Plus className="h-3.5 w-3.5 mr-1" />
          {t("provider.addProvider")}
        </Button>
      </div>

      {/* Provider List */}
      <div className="flex-1 overflow-y-auto px-5 pb-5 space-y-3">
        {loading ? (
          <div className="flex items-center justify-center py-12">
            <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
          </div>
        ) : providers.length === 0 ? (
          <div className="text-center py-12">
            <p className="text-sm text-muted-foreground">
              {t("provider.noProviders")}
            </p>
            <Button
              variant="secondary"
              size="sm"
              className="mt-3"
              onClick={onAddProvider}
            >
              <Plus className="h-3.5 w-3.5 mr-1" />
              {t("provider.addProvider")}
            </Button>
          </div>
        ) : (
          providers.map((provider) => (
            <div
              key={provider.id}
              className={`border rounded-xl p-3.5 transition-colors cursor-pointer ${
                provider.enabled
                  ? "border-border bg-card hover:border-primary/30 hover:bg-card/80"
                  : "border-border/50 bg-card/50 opacity-60 hover:opacity-80"
              }`}
              onClick={() => onEditProvider(provider)}
            >
              <div className="flex items-center gap-3">
                <div className="w-9 h-9 rounded-lg bg-secondary flex items-center justify-center text-muted-foreground shrink-0">
                  <ProviderIcon providerName={provider.name} size={20} color />
                </div>
                <div className="flex-1 min-w-0">
                  <div className="text-sm font-medium text-foreground truncate">
                    {provider.name}
                  </div>
                  <div className="text-[11px] text-muted-foreground flex items-center gap-1.5">
                    <span>{apiTypeLabel(provider.apiType)}</span>
                    <span>·</span>
                    <span>{t("chat.modelsCount", { count: provider.models.length })}</span>
                    {!provider.enabled && (
                      <>
                        <span>·</span>
                        <span className="text-yellow-500">{t("provider.disabled")}</span>
                      </>
                    )}
                  </div>
                </div>

                {/* Action Menu */}
                <div className="relative" onClick={(e) => e.stopPropagation()}>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="h-7 w-7"
                    onClick={() =>
                      setMenuId(menuId === provider.id ? null : provider.id)
                    }
                  >
                    <MoreVertical className="h-3.5 w-3.5" />
                  </Button>
                  {menuId === provider.id && (
                    <>
                      <div
                        className="fixed inset-0 z-40"
                        onClick={() => setMenuId(null)}
                      />
                      <div className="absolute right-0 top-8 z-50 bg-card border border-border rounded-lg shadow-lg py-1 min-w-[130px]">
                        <button
                          className="flex items-center gap-2 w-full px-3 py-1.5 text-xs text-foreground hover:bg-secondary transition-colors"
                          onClick={() => {
                            setMenuId(null)
                            onEditProvider(provider)
                          }}
                        >
                          <Pencil className="h-3 w-3" />
                          {t("common.edit")}
                        </button>
                        <button
                          className="flex items-center gap-2 w-full px-3 py-1.5 text-xs text-foreground hover:bg-secondary transition-colors"
                          onClick={() => toggleProvider(provider)}
                        >
                          {provider.enabled ? (
                            <>
                              <PowerOff className="h-3 w-3" />
                              {t("provider.disable")}
                            </>
                          ) : (
                            <>
                              <Power className="h-3 w-3" />
                              {t("provider.enable")}
                            </>
                          )}
                        </button>
                        {provider.apiType === "codex" && onCodexReauth && (
                          <button
                            className="flex items-center gap-2 w-full px-3 py-1.5 text-xs text-foreground hover:bg-secondary transition-colors"
                            onClick={() => {
                              setMenuId(null)
                              onCodexReauth()
                            }}
                          >
                            <RefreshCw className="h-3 w-3" />
                            {t("provider.relogin")}
                          </button>
                        )}
                        {provider.apiType !== "codex" && (
                          <button
                            className="flex items-center gap-2 w-full px-3 py-1.5 text-xs text-red-400 hover:bg-secondary transition-colors"
                            onClick={() => deleteProvider(provider.id)}
                          >
                            <Trash2 className="h-3 w-3" />
                            {t("common.delete")}
                          </button>
                        )}
                      </div>
                    </>
                  )}
                </div>
              </div>

              {/* Model chips */}
              {provider.models.length > 0 && (
                <div className="flex flex-wrap gap-1.5 mt-2.5">
                  {provider.models.map((model) => (
                    <span
                      key={model.id}
                      className="px-2 py-0.5 text-[10px] rounded-md bg-secondary text-muted-foreground border border-border/50"
                    >
                      {model.name || model.id}
                    </span>
                  ))}
                </div>
              )}
            </div>
          ))
        )}
      </div>
    </div>
  )
}
