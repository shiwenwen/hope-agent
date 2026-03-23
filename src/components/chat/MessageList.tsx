import { useState, useRef, useCallback, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { logger } from "@/lib/logger"
import MessageBubble from "./MessageBubble"
import MessageContextMenu from "./MessageContextMenu"
import type { Message, AgentSummaryForSidebar } from "@/types/chat"

interface MessageListProps {
  messages: Message[]
  setMessages: React.Dispatch<React.SetStateAction<Message[]>>
  loading: boolean
  agents: AgentSummaryForSidebar[]
  hasMore: boolean
  loadingMore: boolean
  onLoadMore: () => void
  scrollContainerRef: React.RefObject<HTMLDivElement | null>
  bottomRef: React.RefObject<HTMLDivElement | null>
  currentSessionId: string | null
  sessionCacheRef: React.MutableRefObject<Map<string, Message[]>>
}

export default function MessageList({
  messages,
  setMessages,
  loading,
  agents,
  hasMore,
  loadingMore,
  onLoadMore,
  scrollContainerRef,
  bottomRef,
  currentSessionId,
  sessionCacheRef,
}: MessageListProps) {
  const { t } = useTranslation()
  const [hoveredMsgIndex, setHoveredMsgIndex] = useState<number | null>(null)
  const [copiedIndex, setCopiedIndex] = useState<number | null>(null)
  const copiedTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const [editingIndex, setEditingIndex] = useState<number | null>(null)
  const [editContent, setEditContent] = useState("")
  const [contextMenu, setContextMenu] = useState<{
    x: number
    y: number
    index: number
  } | null>(null)

  // Close context menu on outside click or scroll
  useEffect(() => {
    if (!contextMenu) return
    const close = () => setContextMenu(null)
    document.addEventListener("mousedown", close)
    document.addEventListener("scroll", close, true)
    return () => {
      document.removeEventListener("mousedown", close)
      document.removeEventListener("scroll", close, true)
    }
  }, [contextMenu])

  function handleContextMenu(e: React.MouseEvent, index: number) {
    const msg = messages[index]
    if (msg.role !== "assistant" || !msg.content) return
    e.preventDefault()
    setContextMenu({ x: e.clientX, y: e.clientY, index })
  }

  function handleCopyMessage(content: string, index: number) {
    navigator.clipboard
      .writeText(content)
      .then(() => {
        if (copiedTimerRef.current) clearTimeout(copiedTimerRef.current)
        setCopiedIndex(index)
        copiedTimerRef.current = setTimeout(() => setCopiedIndex(null), 1500)
      })
      .catch(() => {})
  }

  const handleStartEdit = useCallback(
    (index: number) => {
      const msg = messages[index]
      setEditingIndex(index)
      setEditContent(msg.content)
      setContextMenu(null)
    },
    [messages],
  )

  async function handleSaveEdit(index: number) {
    const msg = messages[index]
    const trimmed = editContent.trim()
    if (!trimmed || trimmed === msg.content) {
      setEditingIndex(null)
      return
    }
    setMessages((prev) => prev.map((m, i) => (i === index ? { ...m, content: trimmed } : m)))
    if (currentSessionId) {
      const cached = sessionCacheRef.current.get(currentSessionId)
      if (cached) {
        sessionCacheRef.current.set(
          currentSessionId,
          cached.map((m, i) => (i === index ? { ...m, content: trimmed } : m)),
        )
      }
    }
    if (msg.dbId) {
      try {
        await invoke("update_message_content_cmd", {
          messageId: msg.dbId,
          content: trimmed,
        })
      } catch (e) {
        logger.error("chat", "MessageList::handleSaveEdit", "Failed to persist edit", { error: e })
      }
    }
    setEditingIndex(null)
  }

  return (
    <div ref={scrollContainerRef} className="flex-1 overflow-y-auto px-4 py-6 space-y-4">
      {/* Load more indicator */}
      {hasMore && (
        <div className="flex justify-center py-2">
          {loadingMore ? (
            <div className="flex items-center gap-2 text-xs text-muted-foreground">
              <div className="h-3.5 w-3.5 animate-spin rounded-full border-2 border-muted-foreground border-t-transparent" />
              {t("chat.loadingMore")}
            </div>
          ) : (
            <button
              onClick={onLoadMore}
              className="text-xs text-muted-foreground hover:text-foreground transition-colors"
            >
              {t("chat.loadMore")}
            </button>
          )}
        </div>
      )}
      {messages.length === 0 && (
        <div className="flex items-center justify-center h-full animate-in fade-in-0 duration-300">
          <p className="text-muted-foreground text-sm">{t("chat.howCanIHelp")}</p>
        </div>
      )}
      {messages.map((msg, i) => (
        <div
          key={i}
          className={cn(
            "flex",
            msg.role === "event" || msg.isSubagentResult
              ? "justify-center"
              : msg.role === "user" && !msg.fromAgentId
                ? "justify-end"
                : "justify-start",
            i === messages.length - 1 && "animate-fade-slide-in",
          )}
        >
          <MessageBubble
            msg={msg}
            index={i}
            isLast={i === messages.length - 1}
            loading={loading}
            agents={agents}
            hoveredMsgIndex={hoveredMsgIndex}
            onHover={setHoveredMsgIndex}
            onContextMenu={handleContextMenu}
            copiedIndex={copiedIndex}
            onCopy={handleCopyMessage}
            editingIndex={editingIndex}
            editContent={editContent}
            onEditContentChange={setEditContent}
            onSaveEdit={handleSaveEdit}
            onCancelEdit={() => setEditingIndex(null)}
          />
        </div>
      ))}
      <div ref={bottomRef} />

      {/* Right-click context menu for assistant messages */}
      {contextMenu && (
        <MessageContextMenu
          contextMenu={contextMenu}
          onStartEdit={handleStartEdit}
          onCopy={(index) => {
            const msg = messages[index]
            if (msg?.content) handleCopyMessage(msg.content, index)
          }}
          onClose={() => setContextMenu(null)}
        />
      )}
    </div>
  )
}
