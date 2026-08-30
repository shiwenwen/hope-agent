import { CircleAlert, CircleCheck, LoaderCircle, MessageSquareText, Plus, X } from "lucide-react"
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"

import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { TRANSPORT_EVENT_RESYNC_REQUIRED } from "@/lib/transport"
import { getTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"
import type { ChatTurnStatus, SessionMeta } from "@/types/chat"
import type { SessionStreamState } from "./hooks/useChatStreamReattach"

type SideChatTrayStatus = "running" | "ready" | "failed"

interface SideChatTurnEvent {
  sessionId?: string
  status?: ChatTurnStatus | null
  turnId?: string | null
  streamId?: string | null
}

function trayStatusForTurn(status: ChatTurnStatus | null | undefined): SideChatTrayStatus | null {
  if (status === "running" || status === "cancelling") return "running"
  if (status === "failed") return "failed"
  if (status === "completed" || status === "interrupted") return "ready"
  return null
}

interface SideChatTrayProps {
  chats: SessionMeta[]
  activeId: string | null
  panelOpen: boolean
  creating: boolean
  onCreate: () => void
  onSelect: (sessionId: string) => void
  onClosePanel: () => void
}

export function SideChatTray({
  chats,
  activeId,
  panelOpen,
  creating,
  onCreate,
  onSelect,
  onClosePanel,
}: SideChatTrayProps) {
  const { t } = useTranslation()
  const [statusBySession, setStatusBySession] = useState<Map<string, SideChatTrayStatus>>(
    () => new Map(),
  )
  const sideChatIdsRef = useRef<Set<string>>(new Set())
  const visibleSideChatIdRef = useRef<string | null>(null)
  const statusVersionRef = useRef<Map<string, number>>(new Map())
  const terminalIdentityRef = useRef<Map<string, string>>(new Map())
  const acknowledgedTerminalRef = useRef<Map<string, string>>(new Map())
  const sideChatIds = useMemo(() => new Set(chats.map((chat) => chat.id)), [chats])

  useLayoutEffect(() => {
    sideChatIdsRef.current = sideChatIds
    visibleSideChatIdRef.current = panelOpen ? activeId : null
  }, [activeId, panelOpen, sideChatIds])

  useEffect(() => {
    for (const sessionId of statusVersionRef.current.keys()) {
      if (!sideChatIds.has(sessionId)) {
        statusVersionRef.current.delete(sessionId)
        terminalIdentityRef.current.delete(sessionId)
        acknowledgedTerminalRef.current.delete(sessionId)
      }
    }

    let cancelled = false
    let refreshVersion = 0
    const transport = getTransport()
    const refreshStatuses = () => {
      const refresh = ++refreshVersion
      for (const chat of chats) {
        const version = statusVersionRef.current.get(chat.id) ?? 0
        void transport
          .call<SessionStreamState>("get_session_stream_state", { sessionId: chat.id })
          .then((state) => {
            if (
              cancelled ||
              refresh !== refreshVersion ||
              !sideChatIdsRef.current.has(chat.id) ||
              (statusVersionRef.current.get(chat.id) ?? 0) !== version
            ) {
              return
            }
            let status: SideChatTrayStatus | null =
              state.active || state.admissionActive
                ? "running"
                : trayStatusForTurn(state.lastTerminalStatus ?? state.status)
            if (status && status !== "running") {
              const identity = state.turnId ?? state.streamId ?? status
              terminalIdentityRef.current.set(chat.id, identity)
              if (state.lastTerminalRead || visibleSideChatIdRef.current === chat.id) {
                acknowledgedTerminalRef.current.set(chat.id, identity)
              }
              if (acknowledgedTerminalRef.current.get(chat.id) === identity) status = null
            }
            setStatusBySession((current) => {
              if ((current.get(chat.id) ?? null) === status) return current
              const next = new Map(current)
              if (status) next.set(chat.id, status)
              else next.delete(chat.id)
              return next
            })
          })
          .catch(() => undefined)
      }
    }
    const unlistenResync = transport.listen(TRANSPORT_EVENT_RESYNC_REQUIRED, refreshStatuses)
    refreshStatuses()
    return () => {
      cancelled = true
      unlistenResync()
    }
  }, [chats, sideChatIds])

  useEffect(() => {
    if (!panelOpen || !activeId) return
    let cancelled = false
    queueMicrotask(() => {
      if (cancelled) return
      const identity = terminalIdentityRef.current.get(activeId)
      if (identity) acknowledgedTerminalRef.current.set(activeId, identity)
      setStatusBySession((current) => {
        if (!current.has(activeId) || current.get(activeId) === "running") return current
        const next = new Map(current)
        next.delete(activeId)
        return next
      })
    })
    return () => {
      cancelled = true
    }
  }, [activeId, panelOpen])

  useEffect(() => {
    const transport = getTransport()
    const updateStatus = (
      sessionId: string | undefined,
      status: SideChatTrayStatus | null,
      turnIdentity?: string | null,
    ) => {
      if (!sessionId || !status || !sideChatIdsRef.current.has(sessionId)) return
      statusVersionRef.current.set(sessionId, (statusVersionRef.current.get(sessionId) ?? 0) + 1)
      if (status === "running") {
        terminalIdentityRef.current.delete(sessionId)
        acknowledgedTerminalRef.current.delete(sessionId)
      } else {
        const identity = turnIdentity ?? status
        terminalIdentityRef.current.set(sessionId, identity)
        if (visibleSideChatIdRef.current === sessionId) {
          acknowledgedTerminalRef.current.set(sessionId, identity)
        }
      }
      const nextStatus =
        status !== "running" &&
        acknowledgedTerminalRef.current.get(sessionId) === terminalIdentityRef.current.get(sessionId)
          ? null
          : status
      setStatusBySession((current) => {
        if (nextStatus && current.get(sessionId) === nextStatus) return current
        if (!nextStatus && !current.has(sessionId)) return current
        const next = new Map(current)
        if (nextStatus) next.set(sessionId, nextStatus)
        else next.delete(sessionId)
        return next
      })
    }
    const unlistenStarted = transport.listen("chat:turn_started", (raw) => {
      const payload = raw as SideChatTurnEvent | null
      updateStatus(payload?.sessionId, "running")
    })
    const updateFromTurnStatus = (raw: unknown) => {
      const payload = raw as SideChatTurnEvent | null
      updateStatus(
        payload?.sessionId,
        trayStatusForTurn(payload?.status),
        payload?.turnId ?? payload?.streamId,
      )
    }
    const unlistenStatus = transport.listen("chat:turn_status", updateFromTurnStatus)
    const unlistenEnd = transport.listen("chat:stream_end", updateFromTurnStatus)
    return () => {
      unlistenStarted()
      unlistenStatus()
      unlistenEnd()
    }
  }, [])

  return (
    <div className="flex min-w-0 items-center gap-1.5 border-b border-border/70 bg-muted/35 px-2 py-1.5">
      <MessageSquareText
        className="h-3.5 w-3.5 shrink-0 text-muted-foreground"
        aria-hidden="true"
      />
      <span className="shrink-0 text-xs font-medium text-muted-foreground">
        {t("chat.sideChat.label", "侧聊")}
      </span>
      <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto">
        {chats.map((chat, index) => {
          const selected = panelOpen && activeId === chat.id
          const status = statusBySession.get(chat.id)
          const title =
            chat.title?.trim() ||
            t("chat.sideChat.untitled", {
              index: index + 1,
              defaultValue: "侧聊 {{index}}",
            })
          const statusLabel = status
            ? t(
                `common.statusValues.${status === "ready" ? "completed" : status}`,
                status === "running" ? "运行中" : status === "ready" ? "已完成" : "失败",
              )
            : null
          return (
            <button
              key={chat.id}
              type="button"
              aria-pressed={selected}
              aria-label={statusLabel ? `${title} · ${statusLabel}` : title}
              onClick={() => {
                const identity = terminalIdentityRef.current.get(chat.id)
                if (identity) acknowledgedTerminalRef.current.set(chat.id, identity)
                setStatusBySession((current) => {
                  if (!current.has(chat.id) || current.get(chat.id) === "running") return current
                  const next = new Map(current)
                  next.delete(chat.id)
                  return next
                })
                onSelect(chat.id)
              }}
              className={cn(
                "flex max-w-40 shrink-0 items-center gap-1 rounded-md px-2 py-1 text-xs transition-colors",
                selected
                  ? "bg-background font-medium text-foreground"
                  : "text-muted-foreground hover:bg-background/70 hover:text-foreground",
              )}
            >
              {status === "running" ? (
                <LoaderCircle
                  className="h-3 w-3 shrink-0 animate-spin text-primary"
                  aria-hidden="true"
                />
              ) : status === "ready" ? (
                <CircleCheck className="h-3 w-3 shrink-0 text-emerald-500" aria-hidden="true" />
              ) : status === "failed" ? (
                <CircleAlert className="h-3 w-3 shrink-0 text-destructive" aria-hidden="true" />
              ) : null}
              <span className="min-w-0 truncate">{title}</span>
            </button>
          )
        })}
      </div>
      <IconTip label={t("chat.sideChat.new", "新建侧聊")}>
        <Button
          type="button"
          size="icon"
          variant="ghost"
          className="h-6 w-6 shrink-0"
          disabled={creating}
          onClick={onCreate}
          aria-label={t("chat.sideChat.new", "新建侧聊")}
        >
          <Plus className="h-3.5 w-3.5" />
        </Button>
      </IconTip>
      {panelOpen ? (
        <IconTip label={t("chat.sideChat.close", "关闭侧聊面板")}>
          <Button
            type="button"
            size="icon"
            variant="ghost"
            className="h-6 w-6 shrink-0"
            onClick={onClosePanel}
            aria-label={t("chat.sideChat.close", "关闭侧聊面板")}
          >
            <X className="h-3.5 w-3.5" />
          </Button>
        </IconTip>
      ) : null}
    </div>
  )
}
