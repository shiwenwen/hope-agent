import { useEffect, useState } from "react"

import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { ChatDisplayMode } from "@/types/chat"
import {
  CHAT_DISPLAY_MODE_EVENT,
  normalizeChatDisplayMode,
  readChatDisplayModePreference,
  writeChatDisplayModePreference,
} from "../chatDisplayModePreference"
import {
  COMPLETED_TURN_COLLAPSE_EVENT,
  normalizeCompletedTurnCollapsePreference,
} from "../completedTurnCollapsePreference"

export interface ChatDisplayPreferences {
  /** 「任务 / 气泡」全局显示模式（设置 → 通用）。 */
  displayMode: ChatDisplayMode
  /** 任务视图下已完成回合是否自动折叠。 */
  autoCollapseCompletedTurns: boolean
}

/**
 * 聊天显示偏好单一入口：localStorage 快照即时初值 → `get_user_config` 权威覆盖
 * （回写 localStorage 保持同步）→ 监听设置页广播事件实时跟随。主聊天
 * （ChatScreen）与知识空间 / 设计空间的内嵌对话面板共用，保证「任务 / 气泡」
 * 模式与回合折叠行为处处一致。
 */
export function useChatDisplayPreferences(): ChatDisplayPreferences {
  const [displayMode, setDisplayMode] = useState<ChatDisplayMode>(() =>
    readChatDisplayModePreference(),
  )
  const [autoCollapseCompletedTurns, setAutoCollapseCompletedTurns] = useState(true)

  useEffect(() => {
    let cancelled = false
    getTransport()
      .call<{ chatDisplayMode?: unknown; autoCollapseCompletedTurns?: unknown }>(
        "get_user_config",
      )
      .then((cfg) => {
        if (cancelled) return
        setAutoCollapseCompletedTurns(
          normalizeCompletedTurnCollapsePreference(cfg.autoCollapseCompletedTurns),
        )
        const mode = normalizeChatDisplayMode(cfg.chatDisplayMode)
        if (mode) {
          setDisplayMode(mode)
          writeChatDisplayModePreference(mode, { emit: false })
        }
      })
      .catch((e: unknown) =>
        logger.warn(
          "settings",
          "useChatDisplayPreferences",
          "Failed to load chat display preferences",
          e,
        ),
      )

    const handleModeChange = (event: Event) => {
      const mode = normalizeChatDisplayMode((event as CustomEvent).detail?.mode)
      if (mode) setDisplayMode(mode)
    }
    const handleCollapseChange = (event: Event) => {
      setAutoCollapseCompletedTurns(
        normalizeCompletedTurnCollapsePreference((event as CustomEvent).detail?.enabled),
      )
    }
    window.addEventListener(CHAT_DISPLAY_MODE_EVENT, handleModeChange)
    window.addEventListener(COMPLETED_TURN_COLLAPSE_EVENT, handleCollapseChange)
    return () => {
      cancelled = true
      window.removeEventListener(CHAT_DISPLAY_MODE_EVENT, handleModeChange)
      window.removeEventListener(COMPLETED_TURN_COLLAPSE_EVENT, handleCollapseChange)
    }
  }, [])

  return { displayMode, autoCollapseCompletedTurns }
}
