/**
 * HandoverDialog — push the current session out to an IM chat.
 *
 * Prefer a recent-conversation picker so users don't need to know the raw
 * chat id. Manual entry remains available for first-time or scripted targets.
 */

import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Loader2, MessageSquare, PencilLine, RefreshCw } from "lucide-react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { IconTip } from "@/components/ui/tooltip"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { ChannelAccountConfig } from "@/components/settings/channel-panel/types"

interface HandoverDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  sessionId: string | null
}

type ChatType = "dm" | "group" | "forum" | "channel"

const CHAT_TYPES: ChatType[] = ["dm", "group", "forum", "channel"]

// Matches the shape returned by `channel_list_sessions`.
interface ChannelConversationDto {
  id: number
  channelId: string
  accountId: string
  chatId: string
  threadId?: string | null
  sessionId: string
  senderId?: string | null
  senderName?: string | null
  chatType: ChatType | string
  createdAt: string
  updatedAt: string
}

type TargetMode = "recent" | "manual"

export default function HandoverDialog({
  open,
  onOpenChange,
  sessionId,
}: HandoverDialogProps) {
  const { t } = useTranslation()
  const [accounts, setAccounts] = useState<ChannelAccountConfig[]>([])
  const [accountId, setAccountId] = useState<string>("")
  const [chatId, setChatId] = useState<string>("")
  const [threadId, setThreadId] = useState<string>("")
  const [chatType, setChatType] = useState<ChatType>("dm")
  const [targetMode, setTargetMode] = useState<TargetMode>("recent")
  const [conversationId, setConversationId] = useState<string>("")
  const [conversations, setConversations] = useState<ChannelConversationDto[]>([])
  const [loadingConversations, setLoadingConversations] = useState(false)
  const [busy, setBusy] = useState(false)
  const [status, setStatus] = useState<"idle" | "saved" | "failed">("idle")
  const [errorMessage, setErrorMessage] = useState<string>("")

  useEffect(() => {
    if (!open) return
    let cancelled = false
    setChatId("")
    setThreadId("")
    setChatType("dm")
    setTargetMode("recent")
    setConversationId("")
    setConversations([])
    setLoadingConversations(false)
    setStatus("idle")
    setErrorMessage("")
    void (async () => {
      try {
        const list = await getTransport().call<ChannelAccountConfig[]>("channel_list_accounts")
        if (cancelled) return
        const accounts = list ?? []
        setAccounts(accounts)
        // Auto-select the first account whenever the dialog opens, so a
        // re-open after the accounts list changed picks something
        // current. The user can still pick another via the Select.
        if (accounts.length > 0) {
          setAccountId(accounts[0].id)
        }
      } catch (e) {
        if (cancelled) return
        logger.warn("chat", "HandoverDialog", "channel_list_accounts failed", e)
        setAccounts([])
      }
    })()
    return () => {
      cancelled = true
    }
  }, [open])

  const selectedAccount = accounts.find((a) => a.id === accountId) ?? null
  const selectedConversation =
    conversations.find((c) => String(c.id) === conversationId) ?? null
  const canSubmit =
    !!sessionId &&
    !!selectedAccount &&
    !busy &&
    (targetMode === "recent"
      ? !!selectedConversation
      : chatId.trim().length > 0)

  useEffect(() => {
    if (!open || !selectedAccount) return
    let cancelled = false

    setConversations([])
    setConversationId("")
    setLoadingConversations(true)

    void (async () => {
      try {
        const list = await getTransport().call<ChannelConversationDto[]>(
          "channel_list_sessions",
          {
            channelId: selectedAccount.channelId,
            accountId: selectedAccount.id,
          },
        )
        if (cancelled) return
        const conversations = list ?? []
        setConversations(conversations)

        if (conversations.length > 0) {
          const first = conversations[0]
          setTargetMode("recent")
          setConversationId(String(first.id))
          applyConversation(first)
        } else {
          setTargetMode("manual")
          setChatId("")
          setThreadId("")
          setChatType("dm")
        }
      } catch (e) {
        if (cancelled) return
        logger.warn("chat", "HandoverDialog", "channel_list_sessions failed", e)
        setConversations([])
        setTargetMode("manual")
      } finally {
        if (!cancelled) setLoadingConversations(false)
      }
    })()

    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, selectedAccount?.id, selectedAccount?.channelId])

  function normalizeChatType(value: string): ChatType {
    return CHAT_TYPES.includes(value as ChatType) ? (value as ChatType) : "dm"
  }

  function conversationLabel(conversation: ChannelConversationDto) {
    const name = conversation.senderName?.trim()
    return name && name.length > 0 ? name : conversation.chatId
  }

  function conversationMeta(conversation: ChannelConversationDto) {
    const type = normalizeChatType(conversation.chatType)
    const parts = [t(`chat.handover.dialog.chatTypes.${type}`)]
    if (conversation.threadId) {
      parts.push(`${t("chat.handover.dialog.threadId")}: ${conversation.threadId}`)
    }
    return parts.join(" · ")
  }

  function applyConversation(conversation: ChannelConversationDto) {
    setChatId(conversation.chatId)
    setThreadId(conversation.threadId ?? "")
    setChatType(normalizeChatType(conversation.chatType))
  }

  function handlePickConversation(nextConversationId: string) {
    setConversationId(nextConversationId)
    const conversation = conversations.find((c) => String(c.id) === nextConversationId)
    if (conversation) applyConversation(conversation)
  }

  async function refreshConversations() {
    if (!selectedAccount || loadingConversations) return
    setLoadingConversations(true)
    try {
      const list = await getTransport().call<ChannelConversationDto[]>(
        "channel_list_sessions",
        {
          channelId: selectedAccount.channelId,
          accountId: selectedAccount.id,
        },
      )
      const conversations = list ?? []
      setConversations(conversations)
      if (conversations.length > 0) {
        const keep =
          conversations.find((c) => String(c.id) === conversationId) ??
          conversations[0]
        setTargetMode("recent")
        setConversationId(String(keep.id))
        applyConversation(keep)
      } else {
        setTargetMode("manual")
        setConversationId("")
      }
    } catch (e) {
      logger.warn("chat", "HandoverDialog", "refresh channel_list_sessions failed", e)
      setConversations([])
      setTargetMode("manual")
    } finally {
      setLoadingConversations(false)
    }
  }

  function switchTargetMode(nextMode: TargetMode) {
    if (nextMode === "recent" && conversations.length === 0) return
    setTargetMode(nextMode)
    if (nextMode === "recent") {
      const conversation = selectedConversation ?? conversations[0]
      if (conversation) {
        setConversationId(String(conversation.id))
        applyConversation(conversation)
      }
    }
  }

  async function handleSubmit() {
    if (!canSubmit || !sessionId || !selectedAccount) return
    const targetChatId =
      targetMode === "recent" ? selectedConversation?.chatId : chatId.trim()
    if (!targetChatId) return
    setBusy(true)
    setStatus("idle")
    setErrorMessage("")
    try {
      await getTransport().call<void>("channel_handover_session", {
        sessionId,
        channelId: selectedAccount.channelId,
        accountId: selectedAccount.id,
        chatId: targetChatId,
        threadId: threadId.trim() ? threadId.trim() : null,
        chatType,
      })
      setStatus("saved")
      setTimeout(() => {
        setStatus("idle")
        onOpenChange(false)
      }, 800)
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e)
      logger.warn("chat", "HandoverDialog", "channel_handover_session failed", e)
      setStatus("failed")
      setErrorMessage(message)
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[480px]">
        <DialogHeader>
          <DialogTitle>{t("chat.handover.dialog.title")}</DialogTitle>
          <DialogDescription>
            {t("chat.handover.dialog.description")}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-3 py-2">
          <div className="space-y-1.5">
            <Label htmlFor="handover-account">
              {t("chat.handover.dialog.account")}
            </Label>
            <Select value={accountId} onValueChange={setAccountId}>
              <SelectTrigger id="handover-account">
                <SelectValue placeholder={t("chat.handover.dialog.selectAccount")} />
              </SelectTrigger>
              <SelectContent>
                {accounts.map((a) => (
                  <SelectItem key={a.id} value={a.id}>
                    {a.label} ({a.channelId})
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="space-y-1.5">
            <div className="flex flex-col gap-1.5 sm:flex-row sm:items-center sm:justify-between">
              <Label htmlFor="handover-conversation">
                {t("chat.handover.dialog.target")}
              </Label>
              <div className="flex items-center gap-1">
                <IconTip label={t("chat.handover.dialog.refreshConversations")}>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    className="h-7 w-7"
                    onClick={refreshConversations}
                    disabled={!selectedAccount || loadingConversations}
                    aria-label={t("chat.handover.dialog.refreshConversations")}
                  >
                    <RefreshCw
                      className={`h-3.5 w-3.5 ${loadingConversations ? "animate-spin" : ""}`}
                    />
                  </Button>
                </IconTip>
                {conversations.length > 0 && (
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="h-7 px-2 text-xs"
                    onClick={() =>
                      switchTargetMode(targetMode === "manual" ? "recent" : "manual")
                    }
                  >
                    {targetMode === "manual" ? (
                      <MessageSquare className="mr-1 h-3.5 w-3.5" />
                    ) : (
                      <PencilLine className="mr-1 h-3.5 w-3.5" />
                    )}
                    {targetMode === "manual"
                      ? t("chat.handover.dialog.chooseRecent")
                      : t("chat.handover.dialog.manualEntry")}
                  </Button>
                )}
              </div>
            </div>

            {targetMode === "recent" && conversations.length > 0 ? (
              <Select value={conversationId} onValueChange={handlePickConversation}>
                <SelectTrigger
                  id="handover-conversation"
                  className="h-auto min-h-9 items-start py-2 text-left [&>span]:line-clamp-none [&>span]:whitespace-normal"
                >
                  <SelectValue placeholder={t("chat.handover.dialog.selectConversation")}>
                    {selectedConversation ? (
                      <span className="flex min-w-0 flex-col gap-0.5">
                        <span className="break-words text-sm leading-5">
                          {conversationLabel(selectedConversation)}
                        </span>
                        <span className="break-words text-xs leading-4 text-muted-foreground">
                          {conversationMeta(selectedConversation)}
                        </span>
                      </span>
                    ) : null}
                  </SelectValue>
                </SelectTrigger>
                <SelectContent>
                  {conversations.map((conversation) => (
                    <SelectItem
                      key={conversation.id}
                      value={String(conversation.id)}
                      className="items-start py-2"
                    >
                      <span className="flex max-w-[min(22rem,calc(100vw-5rem))] flex-col gap-0.5 py-0.5">
                        <span className="whitespace-normal break-words text-sm leading-5">
                          {conversationLabel(conversation)}
                        </span>
                        <span className="whitespace-normal break-words text-xs leading-4 text-muted-foreground">
                          {conversationMeta(conversation)}
                        </span>
                      </span>
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            ) : (
              <Input
                id="handover-chat"
                value={chatId}
                onChange={(e) => setChatId(e.target.value)}
                placeholder={t("chat.handover.dialog.chatIdPlaceholder")}
                autoComplete="off"
                spellCheck={false}
              />
            )}

            {loadingConversations ? (
              <p className="text-xs text-muted-foreground">
                {t("chat.handover.dialog.loadingConversations")}
              </p>
            ) : conversations.length === 0 ? (
              <p className="text-xs text-muted-foreground">
                {t("chat.handover.dialog.noRecentConversations")}
              </p>
            ) : null}
          </div>

          {targetMode === "manual" && (
            <div className="grid grid-cols-2 gap-3">
              <div className="space-y-1.5">
                <Label htmlFor="handover-type">{t("chat.handover.dialog.chatType")}</Label>
                <Select value={chatType} onValueChange={(v) => setChatType(v as ChatType)}>
                  <SelectTrigger id="handover-type">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {CHAT_TYPES.map((type) => (
                      <SelectItem key={type} value={type}>
                        {t(`chat.handover.dialog.chatTypes.${type}`)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="handover-thread">
                  {t("chat.handover.dialog.threadId")}
                </Label>
                <Input
                  id="handover-thread"
                  value={threadId}
                  onChange={(e) => setThreadId(e.target.value)}
                  placeholder={t("chat.handover.dialog.threadIdPlaceholder")}
                  autoComplete="off"
                  spellCheck={false}
                />
              </div>
            </div>
          )}

          {status === "failed" && errorMessage && (
            <p className="text-xs text-destructive">{errorMessage}</p>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={busy}>
            {t("common.cancel")}
          </Button>
          <Button
            onClick={handleSubmit}
            disabled={!canSubmit}
            className={
              status === "saved"
                ? "bg-emerald-600 hover:bg-emerald-600"
                : status === "failed"
                  ? "bg-destructive hover:bg-destructive"
                  : ""
            }
          >
            {busy ? (
              <>
                <Loader2 className="h-3.5 w-3.5 animate-spin mr-1.5" />
                {t("chat.handover.dialog.submitting")}
              </>
            ) : status === "saved" ? (
              t("chat.handover.dialog.submitted")
            ) : (
              t("chat.handover.dialog.submit")
            )}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
