import { useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Check, Loader2 } from "lucide-react"
import type { Transport } from "@/lib/transport"
import { getTransport, useTransport } from "@/lib/transport-provider"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"

export function WeComConnectionFields({
  botId,
  secret,
  onBotIdChange,
  onSecretChange,
  sourceTransport,
}: {
  botId: string
  secret: string
  onBotIdChange: (value: string) => void
  onSecretChange: (value: string) => void
  sourceTransport: Transport
}) {
  const { t } = useTranslation()
  const transport = useTransport()
  const request = useRef(0)
  type Status = "idle" | "checking" | "passed" | "failed"
  const [validation, setValidation] = useState<{
    botId: string
    secret: string
    source: Transport
    status: Status
  } | null>(null)
  const status: Status =
    validation?.botId === botId &&
    validation.secret === secret &&
    validation.source === sourceTransport
      ? validation.status
      : "idle"
  const setStatus = (next: Status) =>
    setValidation({ botId, secret, source: sourceTransport, status: next })
  useEffect(
    () => () => {
      request.current++
    },
    [],
  )
  useEffect(() => {
    request.current++
  }, [botId, secret, sourceTransport])
  const stale = transport !== sourceTransport
  const changed = (update: (value: string) => void, value: string) => {
    request.current++
    setStatus("idle")
    update(value)
  }
  const validate = async () => {
    if (stale || !botId.trim() || !secret.trim()) return
    const generation = ++request.current
    setStatus("checking")
    try {
      await sourceTransport.call("channel_validate_credentials", {
        channelId: "wecom",
        credentials: { botId: botId.trim(), secret: secret.trim() },
      })
      if (request.current === generation && getTransport() === sourceTransport) setStatus("passed")
    } catch {
      // Provider errors and credentials must never be reflected in this form.
      if (request.current === generation && getTransport() === sourceTransport) setStatus("failed")
    }
  }
  return (
    <div className="space-y-3">
      <div className="space-y-2">
        <Label htmlFor="wecom-bot-id">{t("channels.wecomBotId")}</Label>
        <Input
          id="wecom-bot-id"
          value={botId}
          autoComplete="off"
          spellCheck={false}
          onChange={(event) => changed(onBotIdChange, event.target.value)}
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor="wecom-secret">{t("channels.wecomSecret")}</Label>
        <Input
          id="wecom-secret"
          type="password"
          value={secret}
          autoComplete="off"
          onChange={(event) => changed(onSecretChange, event.target.value)}
        />
      </div>
      <p className="text-xs text-muted-foreground">{t("channels.wecomSetupHint")}</p>
      <Button
        variant="outline"
        size="sm"
        onClick={() => void validate()}
        disabled={stale || status === "checking" || !botId.trim() || !secret.trim()}
      >
        {status === "checking" && <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />}
        {status === "passed" && <Check className="mr-1.5 h-3.5 w-3.5" />}
        {t("channels.testConnection")}
      </Button>
      {stale && (
        <p className="text-xs text-destructive" role="alert">
          {t("channels.wecomHostChanged")}
        </p>
      )}
      {status === "passed" && !stale && (
        <p className="text-xs text-green-600">{t("channels.wecomValidated")}</p>
      )}
      {status === "failed" && !stale && (
        <p className="text-xs text-destructive" role="alert">
          {t("channels.wecomValidationFailed")}
        </p>
      )}
    </div>
  )
}

export function WeComGroupFields({
  policy,
  onPolicyChange,
  ids,
  onIdsChange,
}: {
  policy: string
  onPolicyChange: (value: string) => void
  ids: string
  onIdsChange: (value: string) => void
}) {
  const { t } = useTranslation()
  return (
    <div className="space-y-3">
      <Label>{t("channels.groupPolicy")}</Label>
      <Select value={policy} onValueChange={onPolicyChange}>
        <SelectTrigger>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="allowlist">{t("channels.groupPolicyAllowlist")}</SelectItem>
          <SelectItem value="disabled">{t("channels.groupPolicyDisabled")}</SelectItem>
          <SelectItem value="open">{t("channels.groupPolicyOpen")}</SelectItem>
        </SelectContent>
      </Select>
      {policy !== "disabled" && (
        <>
          <Label htmlFor="wecom-groups">{t("channels.wecomGroups")}</Label>
          <Textarea
            id="wecom-groups"
            value={ids}
            onChange={(event) => onIdsChange(event.target.value)}
            rows={3}
          />
          <p className="text-xs text-muted-foreground">{t("channels.wecomGroupsHint")}</p>
        </>
      )}
    </div>
  )
}
