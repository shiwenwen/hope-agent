import { useState, useRef, useCallback } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { logger } from "@/lib/logger"
import type { AvailableModel, ActiveModel } from "@/types/chat"
import { normalizeEffortForModel } from "@/types/chat"
import { toast } from "sonner"

export interface AgentDefaultOption {
  applyToAgentDefault?: boolean
}

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
  handleModelChange: (
    key: string,
    sessionId?: string | null,
    agentId?: string | null,
    options?: AgentDefaultOption,
  ) => Promise<void>
  handleEffortChange: (
    effort: string,
    sessionId?: string | null,
    agentId?: string | null,
    options?: AgentDefaultOption,
  ) => Promise<void>
  handleTemperatureChange: (
    temperature: number,
    sessionId?: string | null,
    agentId?: string | null,
    options?: AgentDefaultOption,
  ) => Promise<void>
  resetSessionEffort: (sessionId: string) => Promise<void>
  resetSessionTemperature: (sessionId: string) => Promise<void>
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
        setReasoningEffort((prev) => normalizeEffortForModel(newModel, prev, t))
      }
    },
    [availableModels, t],
  )

  const handleEffortChange = useCallback(async (
    effort: string,
    sessionId?: string | null,
    agentId?: string | null,
    options?: AgentDefaultOption,
  ) => {
    const previous = reasoningEffort
    setReasoningEffort(effort)
    try {
      if (sessionId) {
        await getTransport().call("set_session_reasoning_effort", {
          sessionId,
          mode: "value",
          value: effort,
        })
      }
    } catch (e) {
      setReasoningEffort(previous)
      logger.error("ui", "ChatScreen::effortChange", "Failed to set reasoning effort", e)
      toast.error(t("common.saveFailed", "保存失败"))
      return
    }
    if (options?.applyToAgentDefault && agentId) {
      try {
        await getTransport().call("patch_agent_model_defaults", {
          id: agentId,
          patch: { reasoningEffort: effort },
        })
      } catch (e) {
        logger.error("ui", "ChatScreen::effortAgentDefault", "Failed to set Agent effort", e)
        toast.error(t("chat.modelPicker.agentDefaultFailed", "当前会话已更新，但 Agent 默认保存失败"))
      }
    }
  }, [reasoningEffort, t])

  // Model selection is Session-local. Agent defaults change only through the
  // explicit popover switch; the global chain is never mutated here.
  const handleModelChange = useCallback(
    async (
      key: string,
      sessionId?: string | null,
      agentId?: string | null,
      options?: AgentDefaultOption,
    ) => {
      const [providerId, modelId] = key.split("::")
      if (!providerId || !modelId) return
      const nextModel = { providerId, modelId }
      const previousModel = activeModel
      setActiveModel(nextModel)
      if (sessionId) {
        try {
          await getTransport().call("set_session_model", { sessionId, providerId, modelId })
        } catch (e) {
          setActiveModel(previousModel)
          logger.error("ui", "ChatScreen::modelChange", "Failed to pin session model", e)
          toast.error(t("common.saveFailed", "保存失败"))
          return
        }
      }
      if (options?.applyToAgentDefault && agentId) {
        try {
          await getTransport().call("patch_agent_model_defaults", {
            id: agentId,
            patch: { primaryModel: nextModel },
          })
        } catch (e) {
          logger.error("ui", "ChatScreen::modelAgentDefault", "Failed to set Agent model", e)
          toast.error(t("chat.modelPicker.agentDefaultFailed", "当前会话已更新，但 Agent 默认保存失败"))
        }
      }

      const newModel = availableModels.find(
        (m) => m.providerId === providerId && m.modelId === modelId,
      )
      if (newModel) {
        const normalized = normalizeEffortForModel(newModel, reasoningEffort, t)
        if (normalized !== reasoningEffort) {
          if (sessionId) {
            await handleEffortChange(normalized, sessionId, agentId)
          } else {
            setReasoningEffort(normalized)
          }
        }
      }
    },
    [activeModel, availableModels, reasoningEffort, t, handleEffortChange],
  )

  const handleTemperatureChange = useCallback(async (
    temperature: number,
    sessionId?: string | null,
    agentId?: string | null,
    options?: AgentDefaultOption,
  ) => {
    const previous = sessionTemperature
    setSessionTemperature(temperature)
    try {
      if (sessionId) {
        await getTransport().call("set_session_temperature", {
          sessionId,
          mode: "value",
          value: temperature,
        })
      }
    } catch (e) {
      setSessionTemperature(previous)
      logger.error("ui", "ChatScreen::temperatureChange", "Failed to set temperature", e)
      toast.error(t("common.saveFailed", "保存失败"))
      return
    }
    if (options?.applyToAgentDefault && agentId) {
      try {
        await getTransport().call("patch_agent_model_defaults", {
          id: agentId,
          patch: { temperature },
        })
      } catch (e) {
        logger.error("ui", "ChatScreen::temperatureAgentDefault", "Failed to set Agent temperature", e)
        toast.error(t("chat.modelPicker.agentDefaultFailed", "当前会话已更新，但 Agent 默认保存失败"))
      }
    }
  }, [sessionTemperature, t])

  const resetSessionEffort = useCallback(async (sessionId: string) => {
    try {
      const effort = await getTransport().call<string>("set_session_reasoning_effort", {
        sessionId,
        mode: "agentDefault",
        value: null,
      })
      setReasoningEffort(effort)
    } catch (e) {
      logger.error("ui", "ChatScreen::effortReset", "Failed to reset reasoning effort", e)
      toast.error(t("common.saveFailed", "保存失败"))
    }
  }, [t])

  const resetSessionTemperature = useCallback(async (sessionId: string) => {
    try {
      const temperature = await getTransport().call<number | null>("set_session_temperature", {
        sessionId,
        mode: "agentDefault",
        value: null,
      })
      setSessionTemperature(temperature)
    } catch (e) {
      logger.error("ui", "ChatScreen::temperatureReset", "Failed to reset temperature", e)
      toast.error(t("common.saveFailed", "保存失败"))
    }
  }, [t])

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
    handleTemperatureChange,
    resetSessionEffort,
    resetSessionTemperature,
  }
}
