import type { ActiveModel, AvailableModel } from "@/types/chat"

function activeModelFromKey(key: string | null | undefined): ActiveModel | null {
  if (!key) return null
  const [providerId, modelId] = key.split("::")
  return providerId && modelId ? { providerId, modelId } : null
}

function isAvailable(models: AvailableModel[], candidate: ActiveModel | null): boolean {
  return Boolean(
    candidate &&
    models.some(
      (model) => model.providerId === candidate.providerId && model.modelId === candidate.modelId,
    ),
  )
}

/** Resolve display state by priority, skipping every stale or disabled candidate. */
export function resolveAvailableDisplayModel(
  availableModels: AvailableModel[],
  sessionPreferred: ActiveModel | null | undefined,
  agentPrimary: string | null | undefined,
  globalActive: ActiveModel | null | undefined,
): ActiveModel | null {
  const candidates = [
    sessionPreferred ?? null,
    activeModelFromKey(agentPrimary),
    globalActive ?? null,
  ]
  return candidates.find((candidate) => isAvailable(availableModels, candidate)) ?? null
}

/** Only explicit user intent may become the backend's strict per-turn override. */
export function modelOverrideFromManualSelection(
  manualSelection: ActiveModel | null | undefined,
): string | undefined {
  return manualSelection ? `${manualSelection.providerId}::${manualSelection.modelId}` : undefined
}
