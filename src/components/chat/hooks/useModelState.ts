import { useState, useRef, useCallback } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import type { AvailableModel, ActiveModel } from "@/types/chat"
import { getEffortOptionsForType } from "@/types/chat"

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
  const applyModelForDisplay = useCallback(
    (key: string) => {
      const [providerId, modelId] = key.split("::")
      if (!providerId || !modelId) return
      setActiveModel({ providerId, modelId })
      const newModel = availableModels.find(
        (m) => m.providerId === providerId && m.modelId === modelId,
      )
      if (newModel) {
        const validOptions = getEffortOptionsForType(newModel.apiType, t)
        const isValid = validOptions.some((opt) => opt.value === reasoningEffort)
        if (!isValid) {
          const fallback = validOptions.some((o) => o.value === "medium") ? "medium" : "none"
          setReasoningEffort(fallback)
        }
      }
    },
    [availableModels, reasoningEffort, t],
  )

  const handleEffortChange = useCallback(async (effort: string) => {
    setReasoningEffort(effort)
    try {
      await getTransport().call("set_reasoning_effort", { effort })
    } catch (e) {
      logger.error("ui", "ChatScreen::effortChange", "Failed to set reasoning effort", e)
    }
  }, [])

  const handleModelChange = useCallback(
    async (key: string) => {
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
        const validOptions = getEffortOptionsForType(newModel.apiType, t)
        const isValid = validOptions.some((opt) => opt.value === reasoningEffort)
        if (!isValid) {
          const fallback = validOptions.some((o) => o.value === "medium") ? "medium" : "none"
          handleEffortChange(fallback)
        }
      }
    },
    [availableModels, reasoningEffort, t, handleEffortChange],
  )

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
