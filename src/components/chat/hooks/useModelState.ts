import { useState, useRef, useCallback } from "react"
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
  handleModelChange: (
    key: string,
    sessionId?: string | null,
    agentId?: string | null,
  ) => Promise<void>
  handleEffortChange: (
    effort: string,
    sessionId?: string | null,
    agentId?: string | null,
  ) => Promise<void>
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
  ) => {
    setReasoningEffort(effort)
    try {
      await getTransport().call("set_reasoning_effort", {
        effort,
        ...(sessionId ? { sessionId } : {}),
        ...(agentId ? { agentId } : {}),
      })
    } catch (e) {
      logger.error("ui", "ChatScreen::effortChange", "Failed to set reasoning effort", e)
    }
  }, [])

  // 用户手动选择的模型同时承担两个作用：更新后续新会话的全局默认，
  // 并在当前会话已经存在时保留该会话自己的模型固定值。
  const handleModelChange = useCallback(
    async (key: string, sessionId?: string | null, agentId?: string | null) => {
      const [providerId, modelId] = key.split("::")
      if (!providerId || !modelId) return
      const nextModel = { providerId, modelId }
      setActiveModel(nextModel)
      globalActiveModelRef.current = nextModel

      const persistGlobalModel = getTransport()
        .call("set_active_model", { providerId, modelId })
        .catch((e) => {
          logger.error("ui", "ChatScreen::modelChange", "Failed to set global active model", e)
        })
      const persistSessionModel = sessionId
        ? getTransport()
            .call("set_session_model", {
              sessionId,
              providerId,
              modelId,
            })
            .catch((e) => {
              logger.error("ui", "ChatScreen::modelChange", "Failed to pin session model", e)
            })
        : Promise.resolve()

      const newModel = availableModels.find(
        (m) => m.providerId === providerId && m.modelId === modelId,
      )
      if (newModel) {
        const normalized = normalizeEffortForModel(newModel, reasoningEffort, t)
        if (normalized !== reasoningEffort) {
          if (sessionId) {
            handleEffortChange(normalized, sessionId, agentId)
          } else if (agentId) {
            handleEffortChange(normalized, null, agentId)
          } else {
            // 草稿模式（会话还没创建）：只更新本地 reasoningEffort，
            // 且没有 agentId 时不调 handleEffortChange，避免把 Think 默认泄漏
            // 到其它会话。
            // 首次发消息时 chat_engine 会把 modelOverride + reasoningEffort 一起带
            // 上去，落到新创建的 sessions 行。
            setReasoningEffort(normalized)
          }
        }
      }

      await Promise.all([persistGlobalModel, persistSessionModel])
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
