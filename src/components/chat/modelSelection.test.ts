import { describe, expect, test } from "vitest"
import type { ActiveModel, AvailableModel } from "@/types/chat"
import { modelOverrideFromManualSelection, resolveAvailableDisplayModel } from "./modelSelection"

function availableModel(providerId: string, modelId: string): AvailableModel {
  return {
    providerId,
    providerName: providerId,
    apiType: "openai_chat",
    modelId,
    modelName: modelId,
    inputTypes: ["text"],
    contextWindow: 128_000,
    maxTokens: 8_192,
    reasoning: false,
  }
}

const sessionModel: ActiveModel = { providerId: "session-provider", modelId: "session-model" }
const agentModel: ActiveModel = { providerId: "agent-provider", modelId: "agent-model" }
const globalModel: ActiveModel = { providerId: "global-provider", modelId: "global-model" }

describe("resolveAvailableDisplayModel", () => {
  test("uses the available session pin before lower-priority candidates", () => {
    const models = [
      availableModel(sessionModel.providerId, sessionModel.modelId),
      availableModel(agentModel.providerId, agentModel.modelId),
      availableModel(globalModel.providerId, globalModel.modelId),
    ]

    expect(
      resolveAvailableDisplayModel(
        models,
        sessionModel,
        `${agentModel.providerId}::${agentModel.modelId}`,
        globalModel,
      ),
    ).toEqual(sessionModel)
  })

  test("falls through an unavailable session pin to an available agent primary", () => {
    const models = [
      availableModel(agentModel.providerId, agentModel.modelId),
      availableModel(globalModel.providerId, globalModel.modelId),
    ]

    expect(
      resolveAvailableDisplayModel(
        models,
        sessionModel,
        `${agentModel.providerId}::${agentModel.modelId}`,
        globalModel,
      ),
    ).toEqual(agentModel)
  })

  test("falls through an unavailable agent primary to an available global active model", () => {
    const models = [availableModel(globalModel.providerId, globalModel.modelId)]

    expect(
      resolveAvailableDisplayModel(
        models,
        sessionModel,
        `${agentModel.providerId}::${agentModel.modelId}`,
        globalModel,
      ),
    ).toEqual(globalModel)
  })

  test("returns null instead of retaining a stale display model when every candidate is unavailable", () => {
    expect(
      resolveAvailableDisplayModel(
        [],
        sessionModel,
        `${agentModel.providerId}::${agentModel.modelId}`,
        globalModel,
      ),
    ).toBeNull()
  })
})

describe("modelOverrideFromManualSelection", () => {
  test("does not turn an automatically resolved display model into a strict override", () => {
    expect(modelOverrideFromManualSelection(null)).toBeUndefined()
  })

  test("serializes a genuine manual selection as the strict override", () => {
    expect(modelOverrideFromManualSelection(agentModel)).toBe(
      `${agentModel.providerId}::${agentModel.modelId}`,
    )
  })
})
