import React, { useRef, useState } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import type { AvailableModel, ActiveModel } from "@/types/chat"
import { normalizeEffortForModel } from "@/types/chat"

export interface UseModelStateReturn {
  availableModels: AvailableModel[]
  setAvailableModels: React.Dispatch<React.SetStateAction<AvailableModel[]>>
  activeModel: ActiveModel | null
  setActiveModel: React.Dispatch<React.SetStateAction<ActiveModel | null>>
  reasoningEffort: string
  setReasoningEffort: React.Dispatch<React.SetStateAction<string>>
  sessionTemperature: number | null
  setSessionTemperature: React.Dispatch<React.SetStateAction<number | null>>
  globalActiveModelRef: React.MutableRefObject<ActiveModel | null>
  applyModelForDisplay: (key: string) => void
  handleModelChange: (key: string) => Promise<void>
  handleEffortChange: (effort: string) => Promise<void>
}

export function useModelState(): UseModelStateReturn {
  const { t } = useTranslation()

  const [availableModels, setAvailableModels] = useState<AvailableModel[]>([])
  const [activeModel, setActiveModel] = useState<ActiveModel | null>(null)
  const [reasoningEffort, setReasoningEffort] = useState("medium")
  const [sessionTemperature, setSessionTemperature] = useState<number | null>(null)
  const globalActiveModelRef = useRef<ActiveModel | null>(null)

  // Update model display + reasoning effort without persisting to global settings
  const applyModelForDisplay = (key: string) => {
    const [providerId, modelId] = key.split("::")
    if (!providerId || !modelId) return
    setActiveModel({ providerId, modelId })
    const newModel = availableModels.find(
      (m) => m.providerId === providerId && m.modelId === modelId,
    )
    if (newModel) {
      setReasoningEffort((prev) => normalizeEffortForModel(newModel, prev, t))
    }
  }

  const handleEffortChange = async (effort: string) => {
    setReasoningEffort(effort)
    try {
      await getTransport().call("set_reasoning_effort", { effort })
    } catch (e) {
      logger.error("ui", "ChatScreen::effortChange", "Failed to set reasoning effort", e)
    }
  }

  const handleModelChange = async (key: string) => {
    const [providerId, modelId] = key.split("::")
    if (!providerId || !modelId) return
    setActiveModel({ providerId, modelId })
    try {
      await getTransport().call("set_active_model", { providerId, modelId })
    } catch (e) {
      logger.error("ui", "ChatScreen::modelChange", "Failed to set model", e)
    }
    const newModel = availableModels.find(
      (m) => m.providerId === providerId && m.modelId === modelId,
    )
    if (newModel) {
      const normalized = normalizeEffortForModel(newModel, reasoningEffort, t)
      if (normalized !== reasoningEffort) {
        handleEffortChange(normalized)
      }
    }
  }

  return {
    availableModels,
    setAvailableModels,
    activeModel,
    setActiveModel,
    reasoningEffort,
    setReasoningEffort,
    sessionTemperature,
    setSessionTemperature,
    globalActiveModelRef,
    applyModelForDisplay,
    handleModelChange,
    handleEffortChange,
  }
}
