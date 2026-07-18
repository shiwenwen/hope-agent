// Settings → Model Configuration → Media Generation Models tab. Draggable provider
// card list (order = failover priority, mirrors ProviderSettings) with an
// add/edit dialog and a confirm-delete flow.

import { useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
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
import { Boxes, GripVertical, Loader2, Pencil, Plus, Trash2 } from "lucide-react"

import { Button } from "@/components/ui/button"
import { Switch } from "@/components/ui/switch"
import { IconTip } from "@/components/ui/tooltip"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import ProviderIcon from "@/components/common/ProviderIcon"
import { logger } from "@/lib/logger"

import { useMediaGenData } from "./useMediaGenData"
import type { MediaProviderConfig } from "./types"
import { VENDOR_DISPLAY_NAME, VENDOR_ICON_KEY } from "./vendor"
import MediaProviderDialog from "./MediaProviderDialog"

// ── Sortable card ─────────────────────────────────────────────────

function SortableMediaProviderCard({
  provider,
  onEdit,
  onToggle,
  onDelete,
}: {
  provider: MediaProviderConfig
  onEdit: (provider: MediaProviderConfig) => void
  onToggle: (provider: MediaProviderConfig, enabled: boolean) => void
  onDelete: (provider: MediaProviderConfig) => void
}) {
  const { t } = useTranslation()
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: provider.id,
  })
  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.4 : 1,
    zIndex: isDragging ? 50 : undefined,
  }
  const hasImage = provider.models.some((m) => m.modality === "image")
  const hasAudio = provider.models.some((m) => m.modality === "audio")

  return (
    <div
      ref={setNodeRef}
      style={style}
      className={`cursor-pointer rounded-xl border p-3.5 transition-colors ${
        provider.enabled
          ? "border-border bg-card hover:bg-secondary/40"
          : "border-border/50 bg-card/50 opacity-60 hover:opacity-80"
      }`}
      onClick={() => onEdit(provider)}
    >
      <div className="flex items-center gap-3">
        <div
          className="shrink-0 cursor-grab touch-none text-muted-foreground/40 active:cursor-grabbing hover:text-muted-foreground/70"
          {...attributes}
          {...listeners}
          onClick={(e) => e.stopPropagation()}
        >
          <GripVertical className="h-4 w-4" />
        </div>
        <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-secondary text-muted-foreground">
          <ProviderIcon providerKey={VENDOR_ICON_KEY[provider.kind]} size={20} color />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="truncate text-sm font-medium text-foreground">{provider.name}</span>
            {hasImage && (
              <span className="shrink-0 rounded-full bg-secondary px-2 py-0.5 text-[10px] font-medium text-secondary-foreground">
                {t("settings.mediaModels.modalityImage")}
              </span>
            )}
            {hasAudio && (
              <span className="shrink-0 rounded-full bg-secondary px-2 py-0.5 text-[10px] font-medium text-secondary-foreground">
                {t("settings.mediaModels.modalityAudio")}
              </span>
            )}
          </div>
          <div className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
            <span>{VENDOR_DISPLAY_NAME[provider.kind]}</span>
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
        <div className="flex shrink-0 items-center gap-1" onClick={(e) => e.stopPropagation()}>
          <Switch
            checked={provider.enabled}
            onCheckedChange={(enabled) => onToggle(provider, enabled)}
            aria-label={t("settings.mediaModels.enabled")}
          />
          <IconTip label={t("common.edit")}>
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7"
              onClick={() => onEdit(provider)}
              aria-label={t("common.edit")}
            >
              <Pencil className="h-3.5 w-3.5" />
            </Button>
          </IconTip>
          <IconTip label={t("common.delete")}>
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7 text-muted-foreground hover:text-destructive"
              onClick={() => onDelete(provider)}
              aria-label={t("common.delete")}
            >
              <Trash2 className="h-3.5 w-3.5" />
            </Button>
          </IconTip>
        </div>
      </div>
      {provider.models.length > 0 && (
        <div className="mt-2.5 flex flex-wrap gap-1.5">
          {provider.models.map((model) => (
            <span
              key={model.id}
              className="rounded-md border border-border/50 bg-secondary px-2 py-0.5 text-[10px] text-muted-foreground"
            >
              {model.name || model.id}
            </span>
          ))}
        </div>
      )}
    </div>
  )
}

// ── Panel ─────────────────────────────────────────────────────────

export default function MediaProvidersPanel() {
  const { t } = useTranslation()
  const {
    config,
    templates,
    loading,
    addProvider,
    updateProvider,
    deleteProvider,
    reorderProviders,
  } = useMediaGenData()
  const [dialog, setDialog] = useState<
    { mode: "create" } | { mode: "edit"; provider: MediaProviderConfig } | null
  >(null)
  const [pendingDelete, setPendingDelete] = useState<MediaProviderConfig | null>(null)

  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 5 } }))
  const providers = config?.providers ?? []

  function handleDragEnd(event: DragEndEvent) {
    const { active, over } = event
    if (!over || active.id === over.id) return
    const oldIndex = providers.findIndex((p) => p.id === active.id)
    const newIndex = providers.findIndex((p) => p.id === over.id)
    if (oldIndex < 0 || newIndex < 0) return
    const updated = arrayMove(providers, oldIndex, newIndex)
    reorderProviders(updated.map((p) => p.id)).catch((e) =>
      logger.error("settings", "MediaProvidersPanel::reorder", "Failed to reorder providers", e),
    )
  }

  async function toggleProvider(provider: MediaProviderConfig, enabled: boolean) {
    try {
      await updateProvider({ ...provider, enabled })
    } catch (e) {
      logger.error("settings", "MediaProvidersPanel::toggle", "Failed to toggle provider", e)
    }
  }

  async function confirmDelete() {
    if (!pendingDelete) return
    const target = pendingDelete
    setPendingDelete(null)
    try {
      const chainsTouched = await deleteProvider(target.id)
      toast.success(t("common.deleted"), {
        description: chainsTouched
          ? t("settings.mediaModels.deleteChainsTouched")
          : target.name,
      })
    } catch (e) {
      logger.error("settings", "MediaProvidersPanel::delete", "Failed to delete provider", e)
      toast.error(t("common.deleteFailed"), { description: target.name })
    }
  }

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center justify-between px-5 pb-2 pt-5">
        <div>
          <h2 className="text-lg font-semibold text-foreground">
            {t("settings.mediaModels.title")}
          </h2>
          {providers.length > 1 && (
            <p className="mt-0.5 text-[10px] text-muted-foreground/60">
              {t("settings.mediaModels.subtitle")}
            </p>
          )}
        </div>
        <Button variant="secondary" size="sm" onClick={() => setDialog({ mode: "create" })}>
          <Plus className="mr-1 h-3.5 w-3.5" />
          {t("settings.mediaModels.addProvider")}
        </Button>
      </div>

      <div className="flex-1 space-y-3 overflow-y-auto px-5 pb-5">
        {loading ? (
          <div className="flex items-center justify-center py-12">
            <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
          </div>
        ) : providers.length === 0 ? (
          <div className="rounded-xl border border-dashed border-border py-12 text-center">
            <Boxes className="mx-auto h-8 w-8 text-muted-foreground/50" />
            <p className="mt-3 text-sm font-medium text-foreground">
              {t("settings.mediaModels.emptyTitle")}
            </p>
            <p className="mx-auto mt-1 max-w-md px-4 text-xs text-muted-foreground">
              {t("settings.mediaModels.emptyDesc")}
            </p>
            <Button
              variant="secondary"
              size="sm"
              className="mt-4"
              onClick={() => setDialog({ mode: "create" })}
            >
              <Plus className="mr-1 h-3.5 w-3.5" />
              {t("settings.mediaModels.addProvider")}
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
                <SortableMediaProviderCard
                  key={provider.id}
                  provider={provider}
                  onEdit={(p) => setDialog({ mode: "edit", provider: p })}
                  onToggle={(p, enabled) => void toggleProvider(p, enabled)}
                  onDelete={setPendingDelete}
                />
              ))}
            </SortableContext>
          </DndContext>
        )}
      </div>

      {dialog && (
        <MediaProviderDialog
          templates={templates}
          provider={dialog.mode === "edit" ? dialog.provider : null}
          onClose={() => setDialog(null)}
          onSubmit={async (provider, isNew) => {
            if (isNew) await addProvider(provider)
            else await updateProvider(provider)
          }}
        />
      )}

      <AlertDialog open={!!pendingDelete} onOpenChange={(open) => !open && setPendingDelete(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("settings.mediaModels.confirmDelete")}</AlertDialogTitle>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={() => void confirmDelete()}
            >
              {t("common.delete")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
