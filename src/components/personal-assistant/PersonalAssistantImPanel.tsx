import { useEffect, useId, useState } from "react"
import { useTranslation } from "react-i18next"
import { Check, Loader2 } from "lucide-react"
import { openExternalUrl } from "@/lib/openExternalUrl"
import { useTransportRevision } from "@/lib/transport-provider"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import type { SessionMeta } from "@/types/chat"
import {
  usePersonalAssistantIm,
  type AssistantImGrant,
  type AssistantImPairingCode,
} from "./usePersonalAssistantIm"

function dateInputValue(date: Date): string {
  return [
    date.getFullYear(),
    String(date.getMonth() + 1).padStart(2, "0"),
    String(date.getDate()).padStart(2, "0"),
  ].join("-")
}

function GrantEditor({
  sessionId,
  grant,
  disabled,
  checkedAt,
  onSave,
}: {
  sessionId: string
  grant?: AssistantImGrant
  disabled: boolean
  checkedAt: number
  onSave: (input: AssistantImGrant) => Promise<void>
}) {
  const { t } = useTranslation()
  const id = useId()
  const [queryStatus, setQueryStatus] = useState(grant?.queryStatus ?? false)
  const [delegate, setDelegate] = useState(grant?.delegate ?? false)
  const [stop, setStop] = useState(grant?.stop ?? false)
  const [completionNotice, setCompletionNotice] = useState(grant?.completionNotice ?? false)
  const [expiry, setExpiry] = useState(() =>
    dateInputValue(new Date(grant ? grant.expiresAt * 1000 : checkedAt + 7 * 86400000)),
  )
  const expiresAt = Math.floor(new Date(expiry + "T23:59:59").getTime() / 1000)
  const expiresValid =
    Number.isFinite(expiresAt) &&
    expiresAt > checkedAt / 1000 &&
    expiresAt <= checkedAt / 1000 + 366 * 86400
  return (
    <form
      className="space-y-3 rounded-md border border-border-soft p-3"
      onSubmit={(event) => {
        event.preventDefault()
        if (!expiresValid || expiresAt <= Date.now() / 1000 || disabled) return
        void onSave({
          sessionId,
          revision: grant?.revision ?? 0,
          queryStatus,
          delegate,
          stop,
          completionNotice,
          expiresAt,
        })
      }}
    >
      <div className="flex items-center justify-between gap-3">
        <Label htmlFor={id + "-query"}>
          {t("personalAssistant.im.queryStatus", "Ask about work status")}
        </Label>
        <Switch
          id={id + "-query"}
          checked={queryStatus}
          disabled={disabled}
          onCheckedChange={(value) => {
            setQueryStatus(value)
            if (!value) {
              setDelegate(false)
              setStop(false)
              setCompletionNotice(false)
            }
          }}
        />
      </div>
      <div className="flex items-center justify-between gap-3">
        <Label htmlFor={id + "-delegate"}>
          {t("personalAssistant.im.delegate", "Delegate new work")}
        </Label>
        <Switch
          id={id + "-delegate"}
          checked={delegate}
          disabled={disabled || !queryStatus}
          onCheckedChange={setDelegate}
        />
      </div>
      <div className="flex items-center justify-between gap-3">
        <Label htmlFor={id + "-stop"}>
          {t("personalAssistant.im.stop", "Stop delegated work")}
        </Label>
        <Switch
          id={id + "-stop"}
          checked={stop}
          disabled={disabled || !queryStatus}
          onCheckedChange={setStop}
        />
      </div>
      <div className="flex items-center justify-between gap-3">
        <Label htmlFor={id + "-notice"}>
          {t("personalAssistant.im.completionNotice", "Receive completion notices")}
        </Label>
        <Switch
          id={id + "-notice"}
          checked={completionNotice}
          disabled={disabled || !queryStatus}
          onCheckedChange={setCompletionNotice}
        />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor={id + "-expiry"}>
          {t("personalAssistant.im.expires", "Permission expires")}
        </Label>
        <Input
          id={id + "-expiry"}
          type="date"
          value={expiry}
          min={dateInputValue(new Date(checkedAt))}
          max={dateInputValue(new Date(checkedAt + 365 * 86400000))}
          onChange={(event) => setExpiry(event.target.value)}
          disabled={disabled}
          required
        />
      </div>
      <p className="text-xs text-muted-foreground">
        {t(
          "personalAssistant.im.statusOnly",
          "Telegram replies show work status. Open Hope to review source messages and results.",
        )}
      </p>
      <Button
        type="submit"
        size="sm"
        disabled={disabled || !expiresValid || (!grant && !queryStatus)}
      >
        {t("common.save")}
      </Button>
    </form>
  )
}

export function PersonalAssistantImPanel({
  sessions,
  visible = true,
}: {
  sessions: SessionMeta[]
  visible?: boolean
}) {
  const revision = useTransportRevision()
  return visible ? <PersonalAssistantImSettings key={revision} sessions={sessions} /> : null
}

function PersonalAssistantImSettings({ sessions }: { sessions: SessionMeta[] }) {
  const { t, i18n } = useTranslation()
  const state = usePersonalAssistantIm()
  const [token, setToken] = useState("")
  const [pairingCode, setPairingCode] = useState<AssistantImPairingCode | null>(null)
  const [selectedSessionId, setSelectedSessionId] = useState("")
  const tokenId = useId()
  const connection = state.data?.connection
  const busy = state.loading || state.pending !== null
  const currentCode =
    connection?.state === "awaiting_code" &&
    pairingCode?.connectionId === connection.connectionId &&
    pairingCode.generation === connection.generation
      ? pairingCode
      : null

  useEffect(() => {
    if (!pairingCode) return
    const expiry = setTimeout(
      () => setPairingCode(null),
      Math.max(0, pairingCode.expiresAt * 1000 - Date.now()),
    )
    return () => clearTimeout(expiry)
  }, [pairingCode])

  const candidates = sessions.filter(
    (session) =>
      (session.kind ?? "regular") === "regular" &&
      !session.incognito &&
      !session.archivedAt &&
      !session.isCron &&
      !session.parentSessionId &&
      !session.channelInfo &&
      session.origin?.kind !== "codex",
  )
  const choices = new Map(candidates.map((session) => [session.id, session.title || session.id]))
  for (const grant of state.data?.grants ?? []) {
    if (!choices.has(grant.sessionId)) choices.set(grant.sessionId, grant.sessionId)
  }
  const selectedGrant = state.data?.grants.find((grant) => grant.sessionId === selectedSessionId)
  const notificationPolicy = state.data?.notificationPolicy
  const formatTime = (seconds: number) => new Date(seconds * 1000).toLocaleString(i18n.language)
  const beginPairing = async () => {
    setPairingCode(null)
    const next = await state.mutate<AssistantImPairingCode>("begin_pairing")
    if (next) setPairingCode(next)
  }

  return (
    <section
      aria-label={t("personalAssistant.im.title", "Connect Telegram")}
      className="space-y-4 p-4"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 className="text-base font-medium">
          {t("personalAssistant.im.title", "Connect Telegram")}
        </h2>
        <Button size="sm" variant="ghost" disabled={busy} onClick={() => void state.refresh()}>
          {t("common.refresh")}
        </Button>
      </div>
      <p className="text-sm text-muted-foreground">
        {t(
          "personalAssistant.im.separateBot",
          "Create a dedicated Telegram bot for assistant chat, then pair your own private chat. Use your work bot for project conversations.",
        )}
      </p>
      {state.pending && (
        <p role="status" className="flex items-center gap-2 text-sm">
          <Loader2 className="size-4 animate-spin" />
          {t("common.saving")}
        </p>
      )}
      {state.feedback === "saved" && (
        <p
          role="status"
          className="flex items-center gap-2 text-sm text-green-700 dark:text-green-400"
        >
          <Check className="size-4" />
          {t("common.saved")}
        </p>
      )}
      {state.error && (
        <p role="alert" className="break-words text-sm text-destructive">
          {state.error.includes("assistant_im_identity_in_use")
            ? t("personalAssistant.im.identityInUse")
            : state.error}
        </p>
      )}
      {state.loading && !state.data && (
        <p role="status" className="text-sm text-muted-foreground">
          {t("common.loading")}
        </p>
      )}

      {state.data && !connection && (
        <form
          className="space-y-2"
          onSubmit={(event) => {
            event.preventDefault()
            if (!token.trim() || busy) return
            void state.mutate("configure", { token: token.trim() }).finally(() => setToken(""))
          }}
        >
          <Label htmlFor={tokenId}>{t("personalAssistant.im.token", "Dedicated bot token")}</Label>
          <Input
            id={tokenId}
            type="password"
            value={token}
            autoComplete="new-password"
            autoCapitalize="none"
            autoCorrect="off"
            spellCheck={false}
            onChange={(event) => setToken(event.target.value)}
            disabled={busy}
          />
          <div>
            <Button
              type="button"
              variant="ghost"
              className="h-auto whitespace-normal px-0 text-left"
              onClick={() =>
                openExternalUrl("https://t.me/BotFather", {
                  onError: () => toast.error(t("common.unavailable")),
                })
              }
            >
              {t("channels.telegramTokenHint")}
            </Button>
          </div>
          <Button type="submit" disabled={busy || !token.trim()}>
            {t("personalAssistant.im.connect", "Verify and connect bot")}
          </Button>
        </form>
      )}

      {connection && (
        <>
          <div className="rounded-md bg-muted/40 p-3 text-sm">
            <p className="break-all font-medium">@{connection.botUsername}</p>
            <p className="text-muted-foreground">
              {t("personalAssistant.im.botId", "Bot ID")} {connection.botId}
            </p>
            <p role="status">
              {t(
                `personalAssistant.im.state.${connection.state}`,
                {
                  unpaired: "Connect your private chat to this bot.",
                  awaiting_code: "Send the pairing command to this bot in your own private chat.",
                  awaiting_confirmation:
                    "A private chat submitted the code. Confirm only after checking that this Telegram identity is yours.",
                  awaiting_activation:
                    "Return to the bot's private chat and press Enable. Only new messages after that step can be accepted.",
                  active: "Your private chat is enabled. Choose the work it may access below.",
                }[connection.state],
              )}
            </p>
            {connection.userId !== null && connection.userId !== undefined && (
              <p>
                {t("personalAssistant.im.userId", "Telegram user ID")} {connection.userId}
              </p>
            )}
            {connection.chatId !== null && connection.chatId !== undefined && (
              <p>
                {t("personalAssistant.im.chatId", "Private chat ID")} {connection.chatId}
              </p>
            )}
          </div>

          {currentCode && (
            <div className="space-y-1 rounded-md border border-border-soft p-3 text-sm">
              <code className="block select-all break-all">/ha_pair {currentCode.code}</code>
              <p className="text-xs text-muted-foreground">
                {t("personalAssistant.im.codeExpires", "Pairing code expires")}{" "}
                {formatTime(currentCode.expiresAt)}
              </p>
            </div>
          )}
          <div className="flex flex-wrap gap-2">
            <Button
              size="sm"
              variant="ghost"
              onClick={() =>
                openExternalUrl("https://t.me/" + encodeURIComponent(connection.botUsername), {
                  onError: () => toast.error(t("common.unavailable")),
                })
              }
            >
              {t("personalAssistant.im.openBot", "Open bot in Telegram")}
            </Button>
            {connection.state === "unpaired" && (
              <Button size="sm" disabled={busy} onClick={() => void beginPairing()}>
                {t("personalAssistant.im.pair", "Generate pairing code")}
              </Button>
            )}
            {(connection.state === "awaiting_code" ||
              connection.state === "awaiting_activation") && (
              <Button
                size="sm"
                variant="outline"
                disabled={busy}
                onClick={() => void beginPairing()}
              >
                {t("personalAssistant.im.restartPairing", "Start pairing again")}
              </Button>
            )}
            {connection.state === "awaiting_confirmation" &&
              connection.userId != null &&
              connection.chatId != null && (
                <Button
                  size="sm"
                  disabled={busy}
                  onClick={() =>
                    void state.mutate("confirm_pairing", {
                      generation: connection.generation,
                      userId: connection.userId,
                      chatId: connection.chatId,
                    })
                  }
                >
                  {t("personalAssistant.im.confirm", "Confirm this Telegram account")}
                </Button>
              )}
            {connection.state !== "unpaired" && (
              <Button
                size="sm"
                variant="outline"
                disabled={busy}
                onClick={() => {
                  setPairingCode(null)
                  void state.mutate("revoke_pairing")
                }}
              >
                {t("personalAssistant.im.unpair", "Unpair private chat")}
              </Button>
            )}
            <Button
              size="sm"
              variant="ghost"
              disabled={busy}
              onClick={() => {
                setPairingCode(null)
                void state.mutate("remove_connection")
              }}
            >
              {t("personalAssistant.im.remove", "Remove assistant bot")}
            </Button>
          </div>

          {connection.state === "active" && (
            <>
              <div className="flex items-center justify-between gap-3">
                <Label htmlFor={tokenId + "-paused"}>
                  {t("personalAssistant.im.paused", "Pause proactive notifications")}
                </Label>
                <Switch
                  id={tokenId + "-paused"}
                  checked={connection.notificationsPaused}
                  disabled={busy}
                  onCheckedChange={(paused) =>
                    void state.mutate("set_notifications_paused", { paused })
                  }
                />
              </div>
              {notificationPolicy && (
                <div className="space-y-1 text-xs text-muted-foreground">
                  <p>
                    {t("personalAssistant.im.notificationPolicy", {
                      start: `${String(notificationPolicy.quietStartHour).padStart(2, "0")}:00`,
                      end: `${String(notificationPolicy.quietEndHour).padStart(2, "0")}:00`,
                      limit: notificationPolicy.dailyLimit,
                    })}
                  </p>
                  <p>
                    {notificationPolicy.timezone
                      ? t("personalAssistant.im.notificationTimezone", {
                          timezone: notificationPolicy.timezone,
                        })
                      : t("personalAssistant.im.notificationTimezoneRequired")}
                  </p>
                </div>
              )}
              <div className="space-y-2">
                <Label htmlFor={tokenId + "-target"}>
                  {t("personalAssistant.im.work", "Work available from this private chat")}
                </Label>
                <Select
                  value={selectedSessionId}
                  onValueChange={setSelectedSessionId}
                  disabled={busy || choices.size === 0}
                >
                  <SelectTrigger id={tokenId + "-target"}>
                    <SelectValue
                      placeholder={t("personalAssistant.im.chooseWork", "Choose a conversation")}
                    />
                  </SelectTrigger>
                  <SelectContent>
                    {Array.from(choices, ([id, title]) => (
                      <SelectItem key={id} value={id}>
                        {title}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                {choices.size === 0 && (
                  <p className="text-sm text-muted-foreground">
                    {t(
                      "personalAssistant.im.noWork",
                      "Open or create a work conversation in Hope first.",
                    )}
                  </p>
                )}
                {selectedSessionId && choices.has(selectedSessionId) && (
                  <GrantEditor
                    key={
                      connection.connectionId +
                      ":" +
                      connection.generation +
                      ":" +
                      selectedSessionId +
                      ":" +
                      (selectedGrant?.revision ?? 0)
                    }
                    sessionId={selectedSessionId}
                    grant={selectedGrant}
                    disabled={busy}
                    checkedAt={state.checkedAt}
                    onSave={async (input) => {
                      await state.mutate("set_grant", { input })
                    }}
                  />
                )}
              </div>
              {(state.data?.grants ?? []).map((grant) => (
                <div
                  key={grant.sessionId}
                  className="flex flex-wrap items-center justify-between gap-2 rounded-md bg-muted/40 p-3 text-sm"
                >
                  <div className="min-w-0">
                    <p className="break-words font-medium">{choices.get(grant.sessionId)}</p>
                    <p className="text-xs text-muted-foreground">
                      {grant.expiresAt <= state.checkedAt / 1000
                        ? t("personalAssistant.im.expired", "Expired")
                        : t("personalAssistant.im.expires", "Permission expires")}{" "}
                      {formatTime(grant.expiresAt)}
                    </p>
                    <p className="text-xs text-muted-foreground">
                      {!grant.queryStatus
                        ? t("personalAssistant.im.noAccess", "Access revoked")
                        : [
                            t("personalAssistant.im.queryStatus", "Ask about work status"),
                            grant.delegate &&
                              t("personalAssistant.im.delegate", "Delegate new work"),
                            grant.stop && t("personalAssistant.im.stop", "Stop delegated work"),
                            grant.completionNotice &&
                              t(
                                "personalAssistant.im.completionNotice",
                                "Receive completion notices",
                              ),
                          ]
                            .filter(Boolean)
                            .join(" · ")}
                    </p>
                  </div>
                  <div className="flex gap-1">
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={busy}
                      onClick={() => setSelectedSessionId(grant.sessionId)}
                    >
                      {t("common.edit")}
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={
                        busy ||
                        !(
                          grant.queryStatus ||
                          grant.delegate ||
                          grant.stop ||
                          grant.completionNotice
                        )
                      }
                      onClick={() =>
                        void state.mutate("revoke_grant", { sessionId: grant.sessionId })
                      }
                    >
                      {t("personalAssistant.im.revoke", "Revoke access")}
                    </Button>
                  </div>
                </div>
              ))}
            </>
          )}
        </>
      )}

      {state.data && state.data.deliveries.length > 0 && (
        <div className="space-y-2">
          <h3 className="text-sm font-medium">
            {t("personalAssistant.im.receipts", "Delivery receipts")}
          </h3>
          <div className="max-h-48 space-y-2 overflow-y-auto">
            {state.data.deliveries.map((delivery) => (
              <div
                key={delivery.id}
                className="flex flex-wrap justify-between gap-2 rounded-md bg-muted/40 px-3 py-2 text-sm"
              >
                <span>
                  {t(
                    `personalAssistant.im.delivery.${delivery.state}`,
                    {
                      pending: "Waiting to send",
                      submitting: "Submitting",
                      accepted: "Platform accepted",
                      unknown: "Outcome unknown",
                      cancelled: "Cancelled",
                    }[delivery.state],
                  )}
                </span>
                <time
                  dateTime={new Date(delivery.createdAt * 1000).toISOString()}
                  className="text-xs text-muted-foreground"
                >
                  {formatTime(delivery.createdAt)}
                </time>
                {delivery.state === "pending" && delivery.deferredReason && (
                  <p className="w-full text-xs text-muted-foreground">
                    {t(`personalAssistant.im.deferred.${delivery.deferredReason}`)}
                  </p>
                )}
              </div>
            ))}
          </div>
        </div>
      )}
    </section>
  )
}
