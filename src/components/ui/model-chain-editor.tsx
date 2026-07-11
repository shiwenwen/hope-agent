import { useState } from "react"
import { useTranslation } from "react-i18next"
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
import { GripVertical, Plus, X } from "lucide-react"
import { ModelSelector, type AvailableModel } from "@/components/ui/model-selector"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"

export interface ModelChainRef {
  primary: { providerId: string; modelId: string }
  fallbacks: { providerId: string; modelId: string }[]
}

export interface ModelChainEditorProps {
  /** `null` = inheriting from whatever this field falls back to. */
  value: ModelChainRef | null
  onChange: (next: ModelChainRef | null) => void
  availableModels: AvailableModel[] | null | undefined
  /** Placeholder shown on the primary selector when `value` is `null`. */
  inheritLabel: string
  /**
   * Show the "add a fallback model" affordance and the sortable fallback
   * list. Set `false` for consumers whose execution genuinely can't use a
   * chain (e.g. Smart mode's judge — deliberately single-shot, no
   * cross-model retry) so the UI doesn't promise resilience that isn't there.
   */
  allowFallbacks?: boolean
  className?: string
}

function modelKey(m: { providerId: string; modelId: string }) {
  return `${m.providerId}::${m.modelId}`
}

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
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id,
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
      className="flex items-center gap-2 px-3 py-2 rounded-lg bg-secondary/30 border border-border/30 group"
    >
      <div
        className="cursor-grab active:cursor-grabbing text-muted-foreground/40 hover:text-muted-foreground/70 shrink-0 touch-none"
        {...attributes}
        {...listeners}
      >
        <GripVertical className="h-3.5 w-3.5" />
      </div>
      <span className="text-[10px] font-bold text-muted-foreground/50 w-5 text-center shrink-0">
        #{index + 1}
      </span>
      <span className="flex-1 text-sm text-foreground truncate">{displayName}</span>
      <Button
        variant="ghost"
        size="icon"
        className="h-6 w-6 text-muted-foreground/40 opacity-0 group-hover:opacity-100 hover:text-destructive"
        onClick={onRemove}
      >
        <X className="h-3.5 w-3.5" />
      </Button>
    </div>
  )
}

/**
 * Reusable "primary model + ordered fallback chain" editor bound to a
 * `ModelChain | null` value (`null` = inherit). Shared by the global
 * automation default (Model Config panel) and every Phase 1 consumer's
 * per-feature override (Recap / Dreaming / Knowledge Compile / Skills
 * auto_review / Hooks `prompt` handler), so "pick a model with degradation"
 * looks and behaves identically everywhere it appears.
 */
export function ModelChainEditor({
  value,
  onChange,
  availableModels,
  inheritLabel,
  allowFallbacks = true,
  className,
}: ModelChainEditorProps) {
  const { t } = useTranslation()
  const [addingFallback, setAddingFallback] = useState(false)
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 5 } }))
  const models = Array.isArray(availableModels) ? availableModels : []

  const modelDisplayName = (ref: { providerId: string; modelId: string }) => {
    const m = models.find(
      (m) => m.providerId === ref.providerId && m.modelId === ref.modelId,
    )
    return m ? `${m.providerName} / ${m.modelName}` : `${ref.providerId}::${ref.modelId}`
  }

  const handlePrimaryChange = (providerId: string, modelId: string) => {
    if (value) {
      onChange({ ...value, primary: { providerId, modelId } })
    } else {
      onChange({ primary: { providerId, modelId }, fallbacks: [] })
    }
  }

  const handleClear = () => onChange(null)

  const fallbacks = value?.fallbacks ?? []

  const handleAddFallback = (providerId: string, modelId: string) => {
    if (!value) return
    if (fallbacks.some((f) => f.providerId === providerId && f.modelId === modelId)) return
    onChange({ ...value, fallbacks: [...fallbacks, { providerId, modelId }] })
    setAddingFallback(false)
  }

  const handleRemoveFallback = (index: number) => {
    if (!value) return
    onChange({ ...value, fallbacks: fallbacks.filter((_, i) => i !== index) })
  }

  const handleFallbackDragEnd = (event: DragEndEvent) => {
    if (!value) return
    const { active, over } = event
    if (!over || active.id === over.id) return
    const oldIndex = fallbacks.findIndex((f) => modelKey(f) === active.id)
    const newIndex = fallbacks.findIndex((f) => modelKey(f) === over.id)
    if (oldIndex === -1 || newIndex === -1) return
    onChange({ ...value, fallbacks: arrayMove(fallbacks, oldIndex, newIndex) })
  }

  const availableForFallback = models.filter(
    (m) =>
      !fallbacks.some((f) => f.providerId === m.providerId && f.modelId === m.modelId) &&
      !(value?.primary.providerId === m.providerId && value?.primary.modelId === m.modelId),
  )

  // Excludes models already in the fallback list — otherwise picking one as
  // primary too produces a chain with the same candidate twice, and
  // automation::run's try-each-in-order loop wastes a retry re-attempting
  // the identical provider+model before reaching any genuinely different
  // fallback.
  const availableForPrimary = models.filter(
    (m) => !fallbacks.some((f) => f.providerId === m.providerId && f.modelId === m.modelId),
  )

  return (
    <div className={className}>
      <div className="flex items-center gap-2">
        <div className="flex-1 min-w-0">
          <ModelSelector
            value={value ? modelKey(value.primary) : ""}
            onChange={handlePrimaryChange}
            availableModels={availableForPrimary}
            placeholder={inheritLabel}
          />
        </div>
        {value && (
          <IconTip label={t("settings.modelChainRestoreInherit")}>
            <Button
              variant="ghost"
              size="icon"
              className="h-9 w-9 shrink-0 text-muted-foreground/50 hover:text-foreground"
              onClick={handleClear}
            >
              <X className="h-3.5 w-3.5" />
            </Button>
          </IconTip>
        )}
      </div>

      {allowFallbacks && value && (
        <div className="mt-2 pl-1">
          {fallbacks.length > 0 && (
            <DndContext
              sensors={sensors}
              collisionDetection={closestCenter}
              onDragEnd={handleFallbackDragEnd}
            >
              <SortableContext
                items={fallbacks.map(modelKey)}
                strategy={verticalListSortingStrategy}
              >
                <div className="space-y-1.5 mb-2">
                  {fallbacks.map((fb, idx) => (
                    <SortableFallbackItem
                      key={modelKey(fb)}
                      id={modelKey(fb)}
                      index={idx}
                      displayName={modelDisplayName(fb)}
                      onRemove={() => handleRemoveFallback(idx)}
                    />
                  ))}
                </div>
              </SortableContext>
            </DndContext>
          )}

          {addingFallback ? (
            <ModelSelector
              defaultOpen={true}
              onOpenChange={(open) => {
                if (!open) setAddingFallback(false)
              }}
              value=""
              onChange={handleAddFallback}
              availableModels={availableForFallback}
              placeholder={t("settings.selectFallbackModel")}
            />
          ) : (
            <Button variant="ghost" size="sm" onClick={() => setAddingFallback(true)}>
              <Plus className="h-3.5 w-3.5" />
              <span>{t("settings.addFallback")}</span>
            </Button>
          )}
        </div>
      )}
    </div>
  )
}
