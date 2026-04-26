import { useState, useEffect } from "react"
import { toast } from "sonner"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import { Button } from "@/components/ui/button"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import type {
  ApiType,
  AuthProfile,
  ModelConfig,
  ThinkingStyleType,
} from "@/components/settings/provider-setup"
import ProviderIcon from "@/components/common/ProviderIcon"
import {
  DndContext,
  closestCenter,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core"
import {
  SortableContext,
  useSortable,
  verticalListSortingStrategy,
  arrayMove,
} from "@dnd-kit/sortable"
import { CSS } from "@dnd-kit/utilities"
import {
  GripVertical,
  Loader2,
  MoreVertical,
  Pencil,
  Plus,
  Power,
  PowerOff,
  RefreshCw,
  Trash2,
} from "lucide-react"
import LocalLlmAssistantCard from "@/components/settings/local-llm/LocalLlmAssistantCard"
import { hasLocalOllamaProvider } from "@/components/settings/local-llm/provider-detection"

// ── Types (shared with ProviderSetup) ─────────────────────────────

export interface ProviderConfig {
  id: string
  name: string
  apiType: ApiType
  baseUrl: string
  apiKey: string
  authProfiles: AuthProfile[]
  models: ModelConfig[]
  enabled: boolean
  userAgent: string
  thinkingStyle: ThinkingStyleType
  allowPrivateNetwork?: boolean
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

// ── Sortable Provider Card ────────────────────────────────────────

function SortableProviderCard({
  provider,
  menuId,
  setMenuId,
  onEditProvider,
  onToggle,
  onDelete,
  onCodexReauth,
  t,
}: {
  provider: ProviderConfig
  menuId: string | null
  setMenuId: (id: string | null) => void
  onEditProvider: (provider: ProviderConfig) => void
  onToggle: (provider: ProviderConfig) => void
  onDelete: (id: string) => void
  onCodexReauth?: () => void
  t: (key: string, opts?: Record<string, unknown>) => string
}) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: provider.id,
  })

  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.4 : 1,
    zIndex: isDragging ? 50 : undefined,
  }

  return (
    <div
      ref={setNodeRef}
      style={style}
      className={`border rounded-xl p-3.5 transition-colors cursor-pointer ${
        provider.enabled
          ? "border-border bg-card hover:border-primary/30 hover:bg-card/80"
          : "border-border/50 bg-card/50 opacity-60 hover:opacity-80"
      }`}
      onClick={() => onEditProvider(provider)}
    >
      <div className="flex items-center gap-3">
        <div
          className="cursor-grab active:cursor-grabbing text-muted-foreground/40 hover:text-muted-foreground/70 shrink-0 touch-none"
          {...attributes}
          {...listeners}
          onClick={(e) => e.stopPropagation()}
        >
          <GripVertical className="h-4 w-4" />
        </div>
        <div className="w-9 h-9 rounded-lg bg-secondary flex items-center justify-center text-muted-foreground shrink-0">
          <ProviderIcon providerName={provider.name} size={20} color />
        </div>
        <div className="flex-1 min-w-0">
          <div className="text-sm font-medium text-foreground truncate">{provider.name}</div>
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
            onClick={() => setMenuId(menuId === provider.id ? null : provider.id)}
          >
            <MoreVertical className="h-3.5 w-3.5" />
          </Button>
          {menuId === provider.id && (
            <>
              <div className="fixed inset-0 z-40" onClick={() => setMenuId(null)} />
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
                  onClick={() => onToggle(provider)}
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
                    onClick={() => onDelete(provider.id)}
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
  )
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
  const [pendingDelete, setPendingDelete] = useState<ProviderConfig | null>(null)

  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 5 } }))

  useEffect(() => {
    loadProviders()
  }, [])

  async function loadProviders() {
    setLoading(true)
    try {
      const list = await getTransport().call<ProviderConfig[]>("get_providers")
      setProviders(list)
    } catch (e) {
      logger.error("settings", "ProviderSettings::load", "Failed to load providers", e)
    } finally {
      setLoading(false)
    }
  }

  async function confirmDeleteProvider() {
    if (!pendingDelete) return
    try {
      await getTransport().call("delete_provider", { providerId: pendingDelete.id })
      await loadProviders()
      toast.success(t("common.deleted"), {
        description: pendingDelete.name,
      })
    } catch (e) {
      logger.error("settings", "ProviderSettings::delete", "Failed to delete provider", e)
      toast.error(t("common.deleteFailed"), {
        description: pendingDelete.name,
      })
    }
    setMenuId(null)
    setPendingDelete(null)
  }

  async function toggleProvider(provider: ProviderConfig) {
    try {
      await getTransport().call("update_provider", {
        config: { ...provider, enabled: !provider.enabled },
      })
      await loadProviders()
    } catch (e) {
      logger.error("settings", "ProviderSettings::toggle", "Failed to toggle provider", e)
    }
    setMenuId(null)
  }

  function handleDragEnd(event: DragEndEvent) {
    const { active, over } = event
    if (!over || active.id === over.id) return
    const oldIndex = providers.findIndex((p) => p.id === active.id)
    const newIndex = providers.findIndex((p) => p.id === over.id)
    const updated = arrayMove(providers, oldIndex, newIndex)
    setProviders(updated)
    getTransport()
      .call("reorder_providers", {
        providerIds: updated.map((p) => p.id),
      })
      .catch((e) =>
        logger.error("settings", "ProviderSettings::reorder", "Failed to reorder providers", e),
      )
  }

  return (
    <div className="flex flex-col h-full">
      {/* Add Provider Button */}
      <div className="flex items-center justify-between px-5 pt-5 pb-2">
        <div>
          <h2 className="text-lg font-semibold text-foreground">{t("provider.title")}</h2>
          {providers.length > 1 && (
            <p className="text-[10px] text-muted-foreground/60 mt-0.5">{t("common.dragToSort")}</p>
          )}
        </div>
        <Button variant="secondary" size="sm" onClick={onAddProvider}>
          <Plus className="h-3.5 w-3.5 mr-1" />
          {t("provider.addProvider")}
        </Button>
      </div>

      {/* Provider List */}
      <div className="flex-1 overflow-y-auto px-5 pb-5 space-y-3">
        {!loading && !hasLocalOllamaProvider(providers) && (
          <LocalLlmAssistantCard onProviderInstalled={() => void loadProviders()} />
        )}
        {loading ? (
          <div className="flex items-center justify-center py-12">
            <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
          </div>
        ) : providers.length === 0 ? (
          <div className="text-center py-12">
            <p className="text-sm text-muted-foreground">{t("provider.noProviders")}</p>
            <Button variant="secondary" size="sm" className="mt-3" onClick={onAddProvider}>
              <Plus className="h-3.5 w-3.5 mr-1" />
              {t("provider.addProvider")}
            </Button>
          </div>
        ) : (
          <DndContext
            sensors={sensors}
            collisionDetection={closestCenter}
            onDragEnd={handleDragEnd}
          >
            <SortableContext
              items={providers.map((p) => p.id)}
              strategy={verticalListSortingStrategy}
            >
              {providers.map((provider) => (
                <SortableProviderCard
                  key={provider.id}
                  provider={provider}
                  menuId={menuId}
                  setMenuId={setMenuId}
                  onEditProvider={onEditProvider}
                  onToggle={toggleProvider}
                  onDelete={(id) => {
                    const provider = providers.find((p) => p.id === id)
                    if (provider) setPendingDelete(provider)
                  }}
                  onCodexReauth={onCodexReauth}
                  t={t}
                />
              ))}
            </SortableContext>
          </DndContext>
        )}
      </div>

      <AlertDialog open={!!pendingDelete} onOpenChange={(open) => !open && setPendingDelete(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("provider.confirmDelete")}</AlertDialogTitle>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={() => void confirmDeleteProvider()}
            >
              {t("common.delete")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
