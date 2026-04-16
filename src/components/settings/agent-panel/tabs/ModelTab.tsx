import { useState } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { Switch } from "@/components/ui/switch"
import { Button } from "@/components/ui/button"
import { ModelSelector } from "@/components/ui/model-selector"
import {
  GripVertical,
  Plus,
  X,
  Lightbulb,
  Thermometer,
  RotateCcw,
} from "lucide-react"
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
import { Slider } from "@/components/ui/slider"
import type { AgentConfig, AvailableModel, ActiveModelRef } from "../types"

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
      className="flex items-center gap-2 px-3 py-2 rounded-lg bg-secondary/40 group"
    >
      <div
        className="cursor-grab active:cursor-grabbing text-muted-foreground/40 hover:text-muted-foreground/70 shrink-0 touch-none"
        {...attributes}
        {...listeners}
      >
        <GripVertical className="h-3.5 w-3.5" />
      </div>
      <span className="text-[10px] px-1.5 py-0.5 rounded bg-primary/10 text-primary font-medium shrink-0">
        #{index + 1}
      </span>
      <span className="text-sm text-foreground flex-1 truncate">
        {displayName}
      </span>
      <button
        className="p-0.5 text-muted-foreground hover:text-destructive transition-colors opacity-0 group-hover:opacity-100"
        onClick={onRemove}
      >
        <X className="h-3 w-3" />
      </button>
    </div>
  )
}

interface ModelTabProps {
  config: AgentConfig
  availableModels: AvailableModel[]
  updateConfig: (patch: Partial<AgentConfig>) => void
}

export default function ModelTab({ config, availableModels, updateConfig }: ModelTabProps) {
  const { t } = useTranslation()
  const [addingAgentFallback, setAddingAgentFallback] = useState(false)
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 5 } }))

  const isCustom = !!config.model.primary
  const modelDisplayName = (ref: string) => {
    const parts = ref.split("::")
    if (parts.length < 2) return ref
    const [pid, ...rest] = parts
    const mid = rest.join("::")
    const m = availableModels.find((m) => m.providerId === pid && m.modelId === mid)
    return m ? `${m.providerName} / ${m.modelName}` : ref
  }
  const fallbacks = config.model.fallbacks || []
  const availableForFallback = availableModels.filter((m) => {
    const ref = `${m.providerId}::${m.modelId}`
    return ref !== config.model.primary && !fallbacks.includes(ref)
  })

  return (
    <div className="space-y-5">
      {/* Inherit / Custom toggle */}
      <div className="flex items-center justify-between px-1">
        <div>
          <div className="text-sm text-foreground">
            {t("settings.agentModelCustom")}
          </div>
          <div className="text-xs text-muted-foreground">
            {t("settings.agentModelCustomDesc")}
          </div>
        </div>
        <Switch
          checked={isCustom}
          onCheckedChange={async (v) => {
            if (v) {
              // Inherit from global settings
              try {
                const [globalActive, globalFallbacks] = await Promise.all([
                  getTransport().call<ActiveModelRef | null>("get_active_model"),
                  getTransport().call<ActiveModelRef[]>("get_fallback_models"),
                ])
                const primary = globalActive
                  ? `${globalActive.providerId}::${globalActive.modelId}`
                  : availableModels[0]
                    ? `${availableModels[0].providerId}::${availableModels[0].modelId}`
                    : null
                const fallbacks = globalFallbacks.map(
                  (f) => `${f.providerId}::${f.modelId}`,
                )
                updateConfig({ model: { ...config.model, primary, fallbacks } })
              } catch {
                // Fallback: use first available model
                const first = availableModels[0]
                if (first) {
                  updateConfig({
                    model: {
                      ...config.model,
                      primary: `${first.providerId}::${first.modelId}`,
                    },
                  })
                }
              }
            } else {
              updateConfig({ model: { primary: null, fallbacks: [] } })
            }
          }}
        />
      </div>

      {!isCustom && (
        <div className="rounded-lg border border-border/50 bg-secondary/20 px-3 py-2">
          <p className="text-xs text-muted-foreground">
            {t("settings.agentModelInheritHint")}
          </p>
        </div>
      )}

      {isCustom && (
        <>
          {/* Primary model selector */}
          <div>
            <div className="text-xs font-medium text-muted-foreground mb-1 px-1">
              {t("settings.agentModelPrimary")}
            </div>
            <ModelSelector
              value={config.model.primary || ""}
              onChange={(providerId, modelId) =>
                updateConfig({
                  model: { ...config.model, primary: `${providerId}::${modelId}` },
                })
              }
              availableModels={availableModels}
              placeholder={t("settings.selectDefaultModel")}
            />
          </div>

          <div className="border-t border-border/50" />

          {/* Fallback models */}
          <div>
            <div className="text-xs font-medium text-muted-foreground mb-1 px-1">
              {t("settings.fallbackModels")}
            </div>
            <p className="text-[11px] text-muted-foreground/60 mb-3 px-1">
              {t("settings.fallbackModelsDesc")}
            </p>

            {fallbacks.length === 0 ? (
              <div className="text-center py-4 text-xs text-muted-foreground/50">
                {t("settings.noFallbackModels")}
              </div>
            ) : (
              <DndContext
                sensors={sensors}
                collisionDetection={closestCenter}
                onDragEnd={(event: DragEndEvent) => {
                  const { active, over } = event
                  if (!over || active.id === over.id) return
                  const oldIndex = fallbacks.indexOf(active.id as string)
                  const newIndex = fallbacks.indexOf(over.id as string)
                  if (oldIndex === -1 || newIndex === -1) return
                  updateConfig({
                    model: { ...config.model, fallbacks: arrayMove(fallbacks, oldIndex, newIndex) },
                  })
                }}
              >
                <SortableContext items={fallbacks} strategy={verticalListSortingStrategy}>
                  <div className="space-y-1 mb-3">
                    {fallbacks.map((ref, i) => (
                      <SortableFallbackItem
                        key={ref}
                        id={ref}
                        index={i}
                        displayName={modelDisplayName(ref)}
                        onRemove={() => {
                          updateConfig({
                            model: {
                              ...config.model,
                              fallbacks: fallbacks.filter((_, j) => j !== i),
                            },
                          })
                        }}
                      />
                    ))}
                  </div>
                </SortableContext>
              </DndContext>
            )}

            {/* Add fallback button / selector */}
            {!addingAgentFallback ? (
              <Button
                variant="ghost"
                size="sm"
                className="gap-1.5 text-primary hover:text-primary/80 px-1"
                onClick={() => setAddingAgentFallback(true)}
              >
                <Plus className="h-3.5 w-3.5" />
                <span>{t("settings.addFallbackModel")}</span>
              </Button>
            ) : (
              <ModelSelector
                defaultOpen={true}
                onOpenChange={(open) => {
                  if (!open) setAddingAgentFallback(false)
                }}
                value=""
                onChange={(providerId, modelId) => {
                  const ref = `${providerId}::${modelId}`
                  updateConfig({
                    model: { ...config.model, fallbacks: [...fallbacks, ref] },
                  })
                  setAddingAgentFallback(false)
                }}
                availableModels={availableForFallback}
                placeholder={t("settings.selectFallbackModel")}
              />
            )}
          </div>
          <div className="border-t border-border/50" />

          {/* Plan Mode model override */}
          <div>
            <div className="flex items-center gap-1.5 mb-1 px-1">
              <Lightbulb className="h-3.5 w-3.5 text-amber-500" />
              <span className="text-xs font-medium text-muted-foreground">
                {t("settings.agentPlanModel")}
              </span>
            </div>
            <p className="text-[11px] text-muted-foreground/60 mb-3 px-1">
              {t("settings.agentPlanModelDesc")}
            </p>

            {config.model.planModel ? (
              <div className="flex items-center gap-2 px-3 py-2 rounded-lg bg-amber-500/5 border border-amber-500/20">
                <span className="text-sm text-foreground flex-1 truncate">
                  {modelDisplayName(config.model.planModel)}
                </span>
                <button
                  className="p-0.5 text-muted-foreground hover:text-destructive transition-colors"
                  onClick={() =>
                    updateConfig({
                      model: { ...config.model, planModel: null },
                    })
                  }
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              </div>
            ) : (
              <ModelSelector
                value=""
                onChange={(providerId, modelId) =>
                  updateConfig({
                    model: { ...config.model, planModel: `${providerId}::${modelId}` },
                  })
                }
                availableModels={availableModels}
                placeholder={t("settings.selectPlanModel")}
              />
            )}
          </div>

          <div className="border-t border-border/50" />

          {/* Temperature override */}
          <div>
            <div className="flex items-center gap-1.5 mb-1 px-1">
              <Thermometer className="h-3.5 w-3.5 text-orange-500" />
              <span className="text-xs font-medium text-muted-foreground">
                {t("settings.temperature")}
              </span>
            </div>
            <p className="text-[11px] text-muted-foreground/60 mb-3 px-1">
              {t("settings.agentTemperatureDesc")}
            </p>

            <div className="flex items-center gap-3 px-1">
              <Slider
                min={0}
                max={200}
                step={1}
                value={[config.model.temperature != null ? Math.round(config.model.temperature * 100) : 100]}
                onValueChange={([v]) => {
                  updateConfig({
                    model: { ...config.model, temperature: v / 100 },
                  })
                }}
                className="flex-1"
              />
              <span className="text-sm font-mono text-foreground w-10 text-right tabular-nums">
                {config.model.temperature != null ? config.model.temperature.toFixed(2) : "1.00"}
              </span>
              <button
                className="text-muted-foreground/50 hover:text-foreground transition-colors"
                onClick={() =>
                  updateConfig({
                    model: { ...config.model, temperature: null },
                  })
                }
                title={t("settings.temperatureReset")}
              >
                <RotateCcw className="h-3.5 w-3.5" />
              </button>
            </div>
          </div>
        </>
      )}
    </div>
  )
}
