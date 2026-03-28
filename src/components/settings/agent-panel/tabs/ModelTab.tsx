import { useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { Switch } from "@/components/ui/switch"
import { Button } from "@/components/ui/button"
import { ModelSelector } from "@/components/ui/model-selector"
import {
  ArrowDown,
  ArrowUp,
  Plus,
  X,
  Lightbulb,
} from "lucide-react"
import type { AgentConfig, AvailableModel, ActiveModelRef } from "../types"

interface ModelTabProps {
  config: AgentConfig
  availableModels: AvailableModel[]
  updateConfig: (patch: Partial<AgentConfig>) => void
}

export default function ModelTab({ config, availableModels, updateConfig }: ModelTabProps) {
  const { t } = useTranslation()
  const [addingAgentFallback, setAddingAgentFallback] = useState(false)

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
                  invoke<ActiveModelRef | null>("get_active_model"),
                  invoke<ActiveModelRef[]>("get_fallback_models"),
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
              <div className="space-y-1 mb-3">
                {fallbacks.map((ref, i) => (
                  <div
                    key={ref}
                    className="flex items-center gap-2 px-3 py-2 rounded-lg bg-secondary/40"
                  >
                    <span className="text-[10px] px-1.5 py-0.5 rounded bg-primary/10 text-primary font-medium shrink-0">
                      #{i + 1}
                    </span>
                    <span className="text-sm text-foreground flex-1 truncate">
                      {modelDisplayName(ref)}
                    </span>
                    <div className="flex items-center gap-0.5 shrink-0">
                      <button
                        className="p-0.5 text-muted-foreground hover:text-foreground transition-colors disabled:opacity-30"
                        onClick={() => {
                          if (i === 0) return
                          const newList = [...fallbacks]
                          ;[newList[i], newList[i - 1]] = [newList[i - 1], newList[i]]
                          updateConfig({
                            model: { ...config.model, fallbacks: newList },
                          })
                        }}
                        disabled={i === 0}
                      >
                        <ArrowUp className="h-3 w-3" />
                      </button>
                      <button
                        className="p-0.5 text-muted-foreground hover:text-foreground transition-colors disabled:opacity-30"
                        onClick={() => {
                          if (i === fallbacks.length - 1) return
                          const newList = [...fallbacks]
                          ;[newList[i], newList[i + 1]] = [newList[i + 1], newList[i]]
                          updateConfig({
                            model: { ...config.model, fallbacks: newList },
                          })
                        }}
                        disabled={i === fallbacks.length - 1}
                      >
                        <ArrowDown className="h-3 w-3" />
                      </button>
                      <button
                        className="p-0.5 text-muted-foreground hover:text-destructive transition-colors ml-1"
                        onClick={() => {
                          updateConfig({
                            model: {
                              ...config.model,
                              fallbacks: fallbacks.filter((_, j) => j !== i),
                            },
                          })
                        }}
                      >
                        <X className="h-3 w-3" />
                      </button>
                    </div>
                  </div>
                ))}
              </div>
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
        </>
      )}
    </div>
  )
}
