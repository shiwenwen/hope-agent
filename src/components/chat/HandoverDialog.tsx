/**
 * HandoverDialog — push the current session out to an IM chat.
 *
 * Surfaces every configured channel-account and asks for a chat id (and
 * optional thread id for forum-style chats). Submitting calls the Phase B1
 * `channel_handover_session` invoke; the backend creates a fresh attach
 * row with `source = "handover"` and promotes it to primary.
 *
 * Kept intentionally lightweight — a fancier picker (history of chats,
 * group / DM toggles, etc.) is a Phase C+ refinement.
 */

import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Loader2 } from "lucide-react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
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
  const [busy, setBusy] = useState(false)
  const [status, setStatus] = useState<"idle" | "saved" | "failed">("idle")
  const [errorMessage, setErrorMessage] = useState<string>("")

  useEffect(() => {
    if (!open) return
    void loadAccounts()
    setChatId("")
    setThreadId("")
    setChatType("dm")
    setStatus("idle")
    setErrorMessage("")
  }, [open])

  async function loadAccounts() {
    try {
      const list = await getTransport().call<ChannelAccountConfig[]>("channel_list_accounts")
      setAccounts(list ?? [])
      if ((list?.length ?? 0) > 0 && !accountId) {
        setAccountId(list![0].id)
      }
    } catch (e) {
      logger.warn("chat", "HandoverDialog", "channel_list_accounts failed", e)
      setAccounts([])
    }
  }

  const selectedAccount = accounts.find((a) => a.id === accountId) ?? null
  const canSubmit =
    !!sessionId && !!selectedAccount && chatId.trim().length > 0 && !busy

  async function handleSubmit() {
    if (!canSubmit || !sessionId || !selectedAccount) return
    setBusy(true)
    setStatus("idle")
    setErrorMessage("")
    try {
      await getTransport().call<void>("channel_handover_session", {
        sessionId,
        channelId: selectedAccount.channelId,
        accountId: selectedAccount.id,
        chatId: chatId.trim(),
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
            <Label htmlFor="handover-chat">
              {t("chat.handover.dialog.chatId")}
            </Label>
            <Input
              id="handover-chat"
              value={chatId}
              onChange={(e) => setChatId(e.target.value)}
              placeholder={t("chat.handover.dialog.chatIdPlaceholder")}
              autoComplete="off"
              spellCheck={false}
            />
          </div>

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
