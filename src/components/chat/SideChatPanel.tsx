import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { MessageSquareText, X } from "lucide-react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"

import ApprovalDialog from "@/components/chat/ApprovalDialog"
import ChatInput from "@/components/chat/ChatInput"
import MessageList from "@/components/chat/MessageList"
import {
  FileActionsContext,
  type FileActionsContextValue,
} from "@/components/chat/files/fileActionsContext"
import type { PreviewTarget } from "@/components/chat/files/useFilePreview"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Button } from "@/components/ui/button"
import { logger } from "@/lib/logger"
import { getTransport } from "@/lib/transport-provider"
import type {
  FileChangeMetadata,
  FileChangesMetadata,
  PendingFileQuote,
  PendingMessageQuote,
} from "@/types/chat"

import { recentUserInputHistory } from "./quick-prompts/messageQuickPrompts"
import { generateClientId } from "./chatScrollKeys"
import { useAskUserPending } from "./ask-user/useAskUserPending"
import type { CommandResult } from "./slash-commands/types"
import { useChatStream } from "./useChatStream"
import { useEmbeddedChatReadReceipt } from "./hooks/useEmbeddedChatReadReceipt"
import { useChatStreamReattach } from "./hooks/useChatStreamReattach"
import { useQuickChatSession } from "./useQuickChatSession"
import type { SubagentRunsSnapshot } from "./subagent/useSubagentRuns"
import type { SubagentOpenTarget } from "./subagent/subagentRunModel"
import { useTaskProgressSnapshot } from "./tasks/useTaskProgressSnapshot"

export interface SideChatSeed {
  nonce: number
  prompt?: string
  quote?: PendingMessageQuote
}

interface SideChatPanelProps {
  sessionId: string
  /** Parent Conversations view visibility; the panel stays mounted across App views. */
  isViewVisible: boolean
  title?: string | null
  workingDir?: string | null
  seed?: SideChatSeed | null
  onClose: () => void
  onActivity?: () => void
  onCodexReauth?: () => void
  onDeleted: (sessionId: string) => void
  onPreviewFile: (target: PreviewTarget) => void
  onFileQuoteHandlerChange?: (handler: ((quote: PendingFileQuote) => void) | null) => void
  onOpenDiff?: (metadata: FileChangeMetadata | FileChangesMetadata) => void
  onOpenSubagentRun?: (target: SubagentOpenTarget) => void
  onViewChildSession?: (sessionId: string) => void
  subagentRunsSnapshot?: SubagentRunsSnapshot
}

export default function SideChatPanel({
  sessionId,
  isViewVisible,
  title,
  workingDir,
  seed,
  onClose,
  onActivity,
  onCodexReauth,
  onDeleted,
  onPreviewFile,
  onFileQuoteHandlerChange,
  onOpenDiff,
  onOpenSubagentRun,
  onViewChildSession,
  subagentRunsSnapshot,
}: SideChatPanelProps) {
  const { t } = useTranslation()
  const session = useQuickChatSession(true, {
    initialSessionId: sessionId,
    persistLastSession: false,
  })
  const streamSeqRef = useRef<Map<string, number>>(new Map())
  const endedStreamIdsRef = useRef<Map<string, Set<string>>>(new Map())
  const consumedSeedRef = useRef(0)
  const [composerFocusSignal, setComposerFocusSignal] = useState<number | undefined>(undefined)
  const [messageTailVisible, setMessageTailVisible] = useState(true)
  const currentSessionMeta = useMemo(
    () =>
      session.currentSessionId
        ? (session.sessions.find((item) => item.id === session.currentSessionId) ?? null)
        : null,
    [session.currentSessionId, session.sessions],
  )
  const { pendingQuestionGroup, setPendingQuestionGroup } = useAskUserPending(
    session.currentSessionId,
  )
  const taskProgressSnapshot = useTaskProgressSnapshot(session.currentSessionId, session.messages)

  useEmbeddedChatReadReceipt(
    isViewVisible,
    messageTailVisible,
    session.currentSessionId,
    session.messages,
  )

  const stream = useChatStream({
    uiSurface: "side_chat",
    messages: session.messages,
    setMessages: session.setMessages,
    currentSessionId: session.currentSessionId,
    setCurrentSessionId: session.setCurrentSessionId,
    currentSessionIdRef: session.currentSessionIdRef,
    currentAgentId: session.currentAgentId,
    agentName: session.agentName,
    loading: session.loading,
    setLoading: session.setLoading,
    loadingSessionsRef: session.loadingSessionsRef,
    setLoadingSessionIds: session.setLoadingSessionIds,
    sessionCacheRef: session.sessionCacheRef,
    sessions: session.sessions,
    agents: session.agents,
    manualModelOverrideRef: session.manualModelOverrideRef,
    reasoningEffort: session.reasoningEffort,
    temperatureOverride: session.sessionTemperature,
    mentionWorkingDir: workingDir ?? null,
    reloadSessions: session.reloadSessions,
    updateSessionMessages: session.updateSessionMessages,
    lastSeqRef: streamSeqRef,
    endedStreamIdsRef,
    incognitoEnabled: false,
    parentInjectionDeltasViaChatStream: true,
  })

  useChatStreamReattach({
    currentSessionId: session.currentSessionId,
    currentSessionIdRef: session.currentSessionIdRef,
    lastSeqRef: streamSeqRef,
    endedStreamIdsRef,
    updateSessionMessages: session.updateSessionMessages,
    setShowCodexAuthExpired: stream.setShowCodexAuthExpired,
    setMessages: session.setMessages,
    setLoading: session.setLoading,
    loadingSessionsRef: session.loadingSessionsRef,
    setLoadingSessionIds: session.setLoadingSessionIds,
    sessionCacheRef: session.sessionCacheRef,
    reloadSessions: session.reloadSessions,
    onTurnStarted: stream.handleTurnStarted,
    onTurnEnded: stream.handleTurnEnded,
  })

  const inputHistory = useMemo(() => recentUserInputHistory(session.messages), [session.messages])

  const replaceDraftAttachment = useCallback(
    (draftId: string, file: File) => {
      stream.setAttachedFiles((current) =>
        current.map((item) =>
          item.id === draftId ? { ...item, file, status: "ready", error: undefined } : item,
        ),
      )
    },
    [stream],
  )
  const fileActionsValue = useMemo<FileActionsContextValue>(
    () => ({
      sessionId: session.currentSessionId ?? sessionId,
      onPreviewFile,
      onReplaceDraft: replaceDraftAttachment,
    }),
    [onPreviewFile, replaceDraftAttachment, session.currentSessionId, sessionId],
  )

  const handleMessageQuote = useCallback(
    (quote: PendingMessageQuote) => {
      stream.setPendingMessageQuotes((current) => [...current, quote])
      setComposerFocusSignal((value) => (value ?? 0) + 1)
    },
    [stream],
  )

  const setPendingQuotes = stream.setPendingQuotes
  const handleFileQuote = useCallback(
    (quote: PendingFileQuote) => {
      setPendingQuotes((current) => [...current, quote])
      setComposerFocusSignal((value) => (value ?? 0) + 1)
    },
    [setPendingQuotes],
  )
  useEffect(() => {
    onFileQuoteHandlerChange?.(handleFileQuote)
    return () => onFileQuoteHandlerChange?.(null)
  }, [handleFileQuote, onFileQuoteHandlerChange])

  const handleSend = useCallback(
    async (prompt?: string) => {
      await stream.handleSend(prompt)
      onActivity?.()
    },
    [onActivity, stream],
  )

  useEffect(() => {
    if (!seed || seed.nonce <= consumedSeedRef.current || session.currentSessionId !== sessionId) {
      return
    }
    let cancelled = false
    queueMicrotask(() => {
      if (cancelled || seed.nonce <= consumedSeedRef.current) return
      consumedSeedRef.current = seed.nonce
      if (seed.quote) handleMessageQuote(seed.quote)
      if (seed.prompt?.trim()) {
        void handleSend(seed.prompt.trim())
      } else {
        setComposerFocusSignal((value) => (value ?? 0) + 1)
      }
    })
    return () => {
      cancelled = true
    }
  }, [handleMessageQuote, handleSend, seed, session.currentSessionId, sessionId])

  const handleCommandAction = useCallback(
    async (result: CommandResult) => {
      const action = result.action
      if (!action) return
      if (action.type === "switchModel") {
        await session.handleModelChange(`${action.providerId}::${action.modelId}`)
      } else if (action.type === "setEffort") {
        await session.handleEffortChange(action.effort)
      } else if (action.type === "showModelPicker") {
        session.setMessages((current) => [
          ...current,
          {
            role: "event",
            content: "",
            timestamp: new Date().toISOString(),
            _clientId: generateClientId(),
            modelPickerData: {
              models: action.models,
              activeProviderId: action.activeProviderId,
              activeModelId: action.activeModelId,
            },
          },
        ])
        onActivity?.()
        return
      } else if (action.type === "stopStream") {
        await stream.handleStop()
        onActivity?.()
        return
      } else if (action.type === "compact") {
        try {
          await getTransport().call("compact_context_now", { sessionId })
        } catch (error) {
          logger.error("ui", "SideChatPanel::compact", "Failed to compact side chat", error)
          toast.error(t("chat.compactFailed"))
        }
      } else if (action.type === "sessionCleared") {
        onDeleted(sessionId)
        return
      }
      await session.reloadMessages(sessionId)
      onActivity?.()
    },
    [onActivity, onDeleted, session, sessionId, stream, t],
  )

  return (
    <aside className="absolute inset-y-0 right-0 z-30 flex w-[min(480px,calc(100%-2rem))] flex-col border-l border-border bg-background shadow-2xl">
      <div className="flex h-11 shrink-0 items-center gap-2 border-b border-border px-3">
        <MessageSquareText className="h-4 w-4 text-primary" aria-hidden="true" />
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-medium">
            {title?.trim() || t("chat.sideChat.label", "侧聊")}
          </div>
          <div className="truncate text-[11px] text-muted-foreground">
            {t("chat.sideChat.nonBlocking", "独立提问，不会中断主对话")}
          </div>
        </div>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="h-7 w-7"
          onClick={onClose}
          aria-label={t("chat.sideChat.close", "关闭侧聊面板")}
        >
          <X className="h-4 w-4" />
        </Button>
      </div>

      <FileActionsContext.Provider value={fileActionsValue}>
        <MessageList
          messages={session.messages}
          loading={session.loading}
          agents={session.agents}
          hasMore={session.hasMore}
          loadingMore={session.loadingMore}
          onLoadMore={session.handleLoadMore}
          sessionId={session.currentSessionId}
          onAddMessageQuote={handleMessageQuote}
          pendingQuestionGroup={pendingQuestionGroup}
          onQuestionSubmitted={() => setPendingQuestionGroup(null)}
          onSwitchModel={(providerId, modelId) => {
            void session.handleModelChange(`${providerId}::${modelId}`)
          }}
          onOpenDiff={onOpenDiff}
          onOpenSubagentRun={onOpenSubagentRun}
          onViewChildSession={onViewChildSession}
          subagentRunsSnapshot={subagentRunsSnapshot}
          onAtBottomChange={setMessageTailVisible}
          onResume={(message) => {
            void handleSend(message)
          }}
        />

        <ApprovalDialog
          requests={stream.approvalRequests}
          onRespond={stream.handleApprovalResponse}
        />

        <AlertDialog
          open={stream.showCodexAuthExpired}
          onOpenChange={stream.setShowCodexAuthExpired}
        >
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>{t("codexAuth.expiredTitle")}</AlertDialogTitle>
              <AlertDialogDescription>{t("codexAuth.expiredDescription")}</AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
              {onCodexReauth && (
                <AlertDialogAction
                  onClick={() => {
                    stream.setShowCodexAuthExpired(false)
                    onCodexReauth()
                  }}
                >
                  {t("codexAuth.reauth")}
                </AlertDialogAction>
              )}
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>

        <div className="shrink-0 border-t border-border px-2 py-2">
          <ChatInput
            input={stream.input}
            structuredMentions={stream.typedMentions}
            onInputChange={stream.setInput}
            onInputChangeWithMention={stream.setInputWithMention}
            inputHistory={inputHistory}
            onSend={() => void handleSend()}
            sendDisabled={session.currentSessionId !== sessionId}
            loading={session.loading}
            availableModels={session.availableModels}
            activeModel={session.activeModel}
            unavailableModelPreference={session.unavailableModelPreference}
            reasoningEffort={session.reasoningEffort}
            onModelChange={session.handleModelChange}
            onEffortChange={session.handleEffortChange}
            onEffortReset={session.resetEffort}
            sessionTemperature={session.sessionTemperature}
            onSessionTemperatureChange={session.handleTemperatureChange}
            attachedFiles={stream.attachedFiles}
            maxAttachmentBytes={stream.maxChatAttachmentBytes}
            onAttachFiles={(files) => stream.setAttachedFiles((current) => [...current, ...files])}
            onRemoveFile={(index) =>
              stream.setAttachedFiles((current) =>
                current.filter((_, itemIndex) => itemIndex !== index),
              )
            }
            onUpdateFile={(index, file) =>
              stream.setAttachedFiles((current) =>
                current.map((item, itemIndex) =>
                  itemIndex === index ? { ...item, file, status: "ready", error: undefined } : item,
                ),
              )
            }
            pendingQuotes={stream.pendingQuotes}
            onRemoveQuote={(index) =>
              stream.setPendingQuotes((current) =>
                current.filter((_, itemIndex) => itemIndex !== index),
              )
            }
            pendingMessageQuotes={stream.pendingMessageQuotes}
            onRemoveMessageQuote={(index) =>
              stream.setPendingMessageQuotes((current) =>
                current.filter((_, itemIndex) => itemIndex !== index),
              )
            }
            focusSignal={composerFocusSignal}
            pendingMessage={stream.pendingMessage}
            pendingSends={stream.pendingSends}
            onCancelPending={() => stream.setPendingMessage(null)}
            onDiscardPending={() => stream.setPendingMessage(null)}
            onEditPending={stream.editPendingSend}
            onDiscardPendingItem={stream.discardPendingSend}
            onSendPending={stream.sendPendingSend}
            onForceInsertPending={stream.forceInsertPendingSend}
            onCancelForceInsertPending={stream.cancelForceInsertPendingSend}
            onStop={stream.handleStop}
            stopPending={stream.stopPendingSessions.has(session.currentSessionId ?? "__pending__")}
            autonomyPaused={currentSessionMeta?.autonomyPaused ?? false}
            onContinue={stream.handleContinue}
            currentSessionId={session.currentSessionId}
            currentAgentId={session.currentAgentId}
            enableAgentMention
            agents={session.agents}
            onCommandAction={handleCommandAction}
            enableGoalAndPlanModes={false}
            enableWorkflowMode={false}
            taskProgressSnapshot={taskProgressSnapshot}
            workingDir={workingDir ?? null}
            permissionMode={stream.permissionMode}
            onPermissionModeChange={stream.setPermissionModeByUser}
            sandboxMode={stream.sandboxMode}
            onSandboxModeChange={stream.setSandboxModeByUser}
          />
        </div>
      </FileActionsContext.Provider>
    </aside>
  )
}
