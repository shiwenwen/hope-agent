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
  handleModelChange: (key: string, sessionId?: string | null) => Promise<void>
  handleEffortChange: (effort: string, sessionId?: string | null) => Promise<void>
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

  const handleEffortChange = useCallback(async (effort: string, sessionId?: string | null) => {
    setReasoningEffort(effort)
    try {
      await getTransport().call("set_reasoning_effort", {
        effort,
        ...(sessionId ? { sessionId } : {}),
      })
    } catch (e) {
      logger.error("ui", "ChatScreen::effortChange", "Failed to set reasoning effort", e)
    }
  }, [])

  // 切换会话内使用的模型：
  // - 已有 sessionId：写到 sessions.provider_id/model_id（只影响这个会话）
  // - 没有 sessionId：仅更新 UI state，等首次发消息时 useChatStream 把
  //   activeModel 作为 modelOverride 透传，chat_engine 的事后 update_session_model
  //   会自动把它落到 sessions 行
  // 全局默认模型现在只能由 Settings 里的 GlobalModelPanel 修改。
  const handleModelChange = useCallback(
    async (key: string, sessionId?: string | null) => {
      const [providerId, modelId] = key.split("::")
      if (!providerId || !modelId) return
      setActiveModel({ providerId, modelId })
      if (sessionId) {
        try {
          await getTransport().call("set_session_model", {
            sessionId,
            providerId,
            modelId,
          })
        } catch (e) {
          logger.error("ui", "ChatScreen::modelChange", "Failed to pin session model", e)
        }
      }
      const newModel = availableModels.find(
        (m) => m.providerId === providerId && m.modelId === modelId,
      )
      if (newModel) {
        const normalized = normalizeEffortForModel(newModel, reasoningEffort, t)
        if (normalized !== reasoningEffort) {
          if (sessionId) {
            handleEffortChange(normalized, sessionId)
          } else {
            // 草稿模式（会话还没创建）：只更新本地 reasoningEffort，
            // 不调 handleEffortChange——后者会把 effort 写到后端的全局 in-memory
            // state，导致一次草稿切模型把 Think 默认泄漏到其它会话。
            // 首次发消息时 chat_engine 会把 modelOverride + reasoningEffort 一起带
            // 上去，落到新创建的 sessions 行。
            setReasoningEffort(normalized)
          }
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
