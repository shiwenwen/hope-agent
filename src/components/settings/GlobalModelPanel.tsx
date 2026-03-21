import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
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
import { GripVertical, Layers, Plus, X } from "lucide-react"
import { ModelSelector } from "@/components/ui/model-selector"
import type { AvailableModel, ActiveModelRef } from "./types"

function SortableFallbackItem({
  id,
  index,
  displayName,
  onRemove,
}: {
  id: string
  index: number
  displayName: string
  onRemove: () => void
}) {
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id })

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
      className="flex items-center gap-2 px-3 py-2 rounded-lg bg-secondary/30 border border-border/30 group"
    >
      {/* Drag handle */}
      <div
        className="cursor-grab active:cursor-grabbing text-muted-foreground/40 hover:text-muted-foreground/70 shrink-0 touch-none"
        {...attributes}
        {...listeners}
      >
        <GripVertical className="h-3.5 w-3.5" />
      </div>

      {/* Priority badge */}
      <span className="text-[10px] font-bold text-muted-foreground/50 w-5 text-center shrink-0">
        #{index + 1}
      </span>

      {/* Model name */}
      <span className="flex-1 text-sm text-foreground truncate">
        {displayName}
      </span>

      {/* Remove */}
      <button
        className="text-muted-foreground/40 hover:text-destructive transition-colors opacity-0 group-hover:opacity-100"
        onClick={onRemove}
      >
        <X className="h-3.5 w-3.5" />
      </button>
    </div>
  )
}

export default function GlobalModelPanel() {
  const { t } = useTranslation()
  const [availableModels, setAvailableModels] = useState<AvailableModel[]>([])
  const [activeModel, setActiveModel] = useState<ActiveModelRef | null>(null)
  const [fallbackModels, setFallbackModels] = useState<ActiveModelRef[]>([])
  const [loading, setLoading] = useState(true)
  const [addingFallback, setAddingFallback] = useState(false)

  useEffect(() => {
    async function load() {
      try {
        const [models, active, fallbacks] = await Promise.all([
          invoke<AvailableModel[]>("get_available_models"),
          invoke<ActiveModelRef | null>("get_active_model"),
          invoke<ActiveModelRef[]>("get_fallback_models"),
        ])
        setAvailableModels(models)
        setActiveModel(active)
        setFallbackModels(fallbacks)
      } catch (e) {
        logger.error("settings", "GlobalModelPanel::load", "Failed to load model settings", e)
      } finally {
        setLoading(false)
      }
    }
    load()
  }, [])

  const modelDisplayName = (ref: ActiveModelRef) => {
    const m = availableModels.find(
      (m) => m.providerId === ref.providerId && m.modelId === ref.modelId
    )
    return m ? `${m.providerName} / ${m.modelName}` : `${ref.providerId}::${ref.modelId}`
  }

  const handleSetDefault = async (providerId: string, modelId: string) => {
    try {
      await invoke("set_active_model", { providerId, modelId })
      setActiveModel({ providerId, modelId })
    } catch (e) {
      logger.error("settings", "GlobalModelPanel::setDefault", "Failed to set default model", e)
    }
  }

  const handleSaveFallbacks = async (newFallbacks: ActiveModelRef[]) => {
    try {
      await invoke("set_fallback_models", { models: newFallbacks })
      setFallbackModels(newFallbacks)
    } catch (e) {
      logger.error("settings", "GlobalModelPanel::saveFallbacks", "Failed to save fallback models", e)
    }
  }

  const handleAddFallback = (providerId: string, modelId: string) => {
    // Avoid duplicates
    if (fallbackModels.some((f) => f.providerId === providerId && f.modelId === modelId)) return
    const newList = [...fallbackModels, { providerId, modelId }]
    handleSaveFallbacks(newList)
    setAddingFallback(false)
  }

  const handleRemoveFallback = (index: number) => {
    const newList = fallbackModels.filter((_, i) => i !== index)
    handleSaveFallbacks(newList)
  }

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 5 } }),
  )

  const handleFallbackDragEnd = (event: DragEndEvent) => {
    const { active, over } = event
    if (!over || active.id === over.id) return
    const oldIndex = fallbackModels.findIndex(
      (f) => `${f.providerId}::${f.modelId}` === active.id
    )
    const newIndex = fallbackModels.findIndex(
      (f) => `${f.providerId}::${f.modelId}` === over.id
    )
    if (oldIndex === -1 || newIndex === -1) return
    const updated = arrayMove(fallbackModels, oldIndex, newIndex)
    handleSaveFallbacks(updated)
  }

  // Available for adding as fallback (not already in list, not the active model)
  const availableForFallback = availableModels.filter(
    (m) =>
      !fallbackModels.some(
        (f) => f.providerId === m.providerId && f.modelId === m.modelId
      ) &&
      !(activeModel?.providerId === m.providerId && activeModel?.modelId === m.modelId)
  )

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <div className="animate-spin h-5 w-5 border-2 border-foreground border-t-transparent rounded-full" />
      </div>
    )
  }

  return (
    <div className="flex-1 overflow-y-auto p-6 max-w-4xl">
      <h2 className="text-lg font-semibold text-foreground mb-1">
        {t("settings.globalModel")}
      </h2>
      <p className="text-xs text-muted-foreground mb-5">
        {t("settings.globalModelDesc")}
      </p>

      {/* Default Model */}
      <div className="mb-6">
        <div className="text-xs font-medium text-muted-foreground mb-1 px-1">
          {t("settings.defaultModel")}
        </div>
        <p className="text-[11px] text-muted-foreground/60 mb-2 px-1">
          {t("settings.defaultModelDesc")}
        </p>

        <ModelSelector
          value={activeModel ? `${activeModel.providerId}::${activeModel.modelId}` : ""}
          onChange={(providerId, modelId) => handleSetDefault(providerId, modelId)}
          availableModels={availableModels}
          placeholder={t("settings.selectDefaultModel")}
        />
      </div>

      <div className="border-t border-border/50 mb-6" />

      {/* Fallback Models */}
      <div>
        <div className="text-xs font-medium text-muted-foreground mb-1 px-1">
          {t("settings.fallbackModels")}
        </div>
        <p className="text-[11px] text-muted-foreground/60 mb-3 px-1">
          {t("settings.fallbackModelsDesc")}
        </p>

        {fallbackModels.length === 0 ? (
          <div className="text-center py-6 bg-secondary/20 rounded-lg border border-border/30">
            <Layers className="h-8 w-8 text-muted-foreground/20 mx-auto mb-2" />
            <p className="text-xs text-muted-foreground/60">
              {t("settings.noFallbacks")}
            </p>
          </div>
        ) : (
          <DndContext
            sensors={sensors}
            collisionDetection={closestCenter}
            onDragEnd={handleFallbackDragEnd}
          >
            <SortableContext
              items={fallbackModels.map((f) => `${f.providerId}::${f.modelId}`)}
              strategy={verticalListSortingStrategy}
            >
              <div className="space-y-1.5 mb-3">
                {fallbackModels.map((fb, idx) => (
                  <SortableFallbackItem
                    key={`${fb.providerId}::${fb.modelId}`}
                    id={`${fb.providerId}::${fb.modelId}`}
                    index={idx}
                    displayName={modelDisplayName(fb)}
                    onRemove={() => handleRemoveFallback(idx)}
                  />
                ))}
              </div>
            </SortableContext>
          </DndContext>
        )}

        {/* Add fallback */}
        {addingFallback ? (
          <ModelSelector
            defaultOpen={true}
            onOpenChange={(open) => {
              if (!open) setAddingFallback(false)
            }}
            value=""
            onChange={(providerId, modelId) => handleAddFallback(providerId, modelId)}
            availableModels={availableForFallback}
            placeholder={t("settings.selectFallbackModel")}
          />

        ) : (
          <button
            className="flex items-center gap-1.5 text-xs text-primary hover:text-primary/80 transition-colors px-1 py-1.5"
            onClick={() => setAddingFallback(true)}
          >
            <Plus className="h-3.5 w-3.5" />
            <span>{t("settings.addFallback")}</span>
          </button>
        )}
      </div>
    </div>
  )
}
