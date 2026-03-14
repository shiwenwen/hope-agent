import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { ModelEditor, type ModelConfig } from "@/components/ProviderSetup"
import ProviderIcon from "@/components/ProviderIcon"
import {
  ArrowLeft,
  Check,
  Globe,
  Key,
  Loader2,
  MoreVertical,
  Pencil,
  Plus,
  Power,
  PowerOff,
  Trash2,
  X,
} from "lucide-react"

// ── Types (shared with ProviderSetup) ─────────────────────────────

type ApiType = "anthropic" | "openai-chat" | "openai-responses" | "codex"

// ModelConfig is imported from ProviderSetup

interface ProviderConfig {
  id: string
  name: string
  apiType: ApiType
  baseUrl: string
  apiKey: string
  models: ModelConfig[]
  enabled: boolean
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
  onBack,
  onAddProvider,
}: {
  onBack: () => void
  onAddProvider: () => void
}) {
  const { t } = useTranslation()
  const [providers, setProviders] = useState<ProviderConfig[]>([])
  const [loading, setLoading] = useState(true)
  const [editingId, setEditingId] = useState<string | null>(null)
  const [menuId, setMenuId] = useState<string | null>(null)

  // Edit form state
  const [editName, setEditName] = useState("")
  const [editBaseUrl, setEditBaseUrl] = useState("")
  const [editApiKey, setEditApiKey] = useState("")
  const [editModels, setEditModels] = useState<ModelConfig[]>([])
  const [saving, setSaving] = useState(false)

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

  function startEdit(provider: ProviderConfig) {
    setEditingId(provider.id)
    setEditName(provider.name)
    setEditBaseUrl(provider.baseUrl)
    setEditApiKey(provider.apiKey)
    setEditModels([...provider.models])
    setMenuId(null)
  }

  async function saveEdit(provider: ProviderConfig) {
    setSaving(true)
    try {
      await invoke("update_provider", {
        config: {
          ...provider,
          name: editName,
          baseUrl: editBaseUrl,
          apiKey: editApiKey || provider.apiKey, // Keep old key if empty
          models: editModels,
        },
      })
      await loadProviders()
      setEditingId(null)
    } catch (e) {
      console.error("Failed to update provider:", e)
    } finally {
      setSaving(false)
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
      {/* Header */}
      <div className="h-11 flex items-center justify-between px-4 border-b border-border shrink-0">
        <button
          onClick={onBack}
          className="flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground transition-colors"
        >
          <ArrowLeft className="h-4 w-4" />
          {t("common.back")}
        </button>
        <span className="text-sm font-semibold text-foreground">
          {t("provider.title")}
        </span>
        <Button variant="ghost" size="sm" onClick={onAddProvider}>
          <Plus className="h-3.5 w-3.5 mr-1" />
          {t("common.add")}
        </Button>
      </div>

      {/* Provider List */}
      <div className="flex-1 overflow-y-auto p-4 space-y-3">
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
          providers.map((provider) =>
            editingId === provider.id ? (
              // ── Edit Mode ──
              <div
                key={provider.id}
                className="border border-primary/30 rounded-xl p-4 space-y-3 bg-card"
              >
                <div className="flex items-center justify-between">
                  <span className="text-xs font-medium text-primary">
                    {t("provider.editProvider")}
                  </span>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="h-6 w-6"
                    onClick={() => setEditingId(null)}
                  >
                    <X className="h-3.5 w-3.5" />
                  </Button>
                </div>

                <div className="space-y-2.5">
                  <div className="space-y-1">
                    <label className="text-[10px] text-muted-foreground">
                      {t("provider.name")}
                    </label>
                    <Input
                      value={editName}
                      onChange={(e) => setEditName(e.target.value)}
                      className="bg-background text-xs h-8"
                    />
                  </div>
                  {provider.apiType !== "codex" && (
                    <>
                      <div className="space-y-1">
                        <label className="text-[10px] text-muted-foreground flex items-center gap-1">
                          <Globe className="h-2.5 w-2.5" />
                          Base URL
                        </label>
                        <Input
                          value={editBaseUrl}
                          onChange={(e) => setEditBaseUrl(e.target.value)}
                          className="bg-background text-xs h-8 font-mono"
                        />
                      </div>
                      <div className="space-y-1">
                        <label className="text-[10px] text-muted-foreground flex items-center gap-1">
                          <Key className="h-2.5 w-2.5" />
                          {t("provider.apiKeyLeaveEmpty")}
                        </label>
                        <Input
                          type="password"
                          value={editApiKey}
                          onChange={(e) => setEditApiKey(e.target.value)}
                          placeholder={t("provider.leaveEmptyNoChange")}
                          className="bg-background text-xs h-8 font-mono"
                        />
                      </div>
                    </>
                  )}
                </div>

                {/* Models editor - reuse ModelEditor from ProviderSetup */}
                <div className="space-y-2">
                  <div className="flex items-center justify-between">
                    <span className="text-[10px] text-muted-foreground font-medium">
                      {t("model.modelList")}
                    </span>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-6 text-[10px]"
                      onClick={() =>
                        setEditModels([
                          ...editModels,
                          {
                            id: "",
                            name: "",
                            inputTypes: ["text"],
                            contextWindow: 200000,
                            maxTokens: 8192,
                            reasoning: false,
                            costInput: 0,
                            costOutput: 0,
                          },
                        ])
                      }
                    >
                      <Plus className="h-2.5 w-2.5 mr-0.5" />
                      {t("model.addModel")}
                    </Button>
                  </div>
                  {editModels.map((model, i) => (
                    <ModelEditor
                      key={i}
                      model={model}
                      onChange={(updated) => {
                        const next = [...editModels]
                        next[i] = updated
                        setEditModels(next)
                      }}
                      onRemove={() =>
                        setEditModels(editModels.filter((_, j) => j !== i))
                      }
                    />
                  ))}
                </div>

                <div className="flex justify-end gap-2">
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() => setEditingId(null)}
                  >
                    {t("common.cancel")}
                  </Button>
                  <Button
                    size="sm"
                    onClick={() => saveEdit(provider)}
                    disabled={saving}
                  >
                    {saving ? (
                      <Loader2 className="h-3 w-3 animate-spin" />
                    ) : (
                      <>
                        <Check className="h-3 w-3 mr-1" />
                        {t("common.save")}
                      </>
                    )}
                  </Button>
                </div>
              </div>
            ) : (
              // ── Display Mode ──
              <div
                key={provider.id}
                className={`border rounded-xl p-3.5 transition-colors ${
                  provider.enabled
                    ? "border-border bg-card"
                    : "border-border/50 bg-card/50 opacity-60"
                }`}
              >
                <div className="flex items-center gap-3">
                  <div className="w-9 h-9 rounded-lg bg-secondary flex items-center justify-center text-muted-foreground shrink-0">
                    <ProviderIcon providerName={provider.name} size={20} />
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
                  <div className="relative">
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
                            onClick={() => startEdit(provider)}
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
            ),
          )
        )}
      </div>
    </div>
  )
}
