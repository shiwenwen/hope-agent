import { useState, useEffect, useRef } from "react"
import { useTranslation } from "react-i18next"
import { invoke } from "@tauri-apps/api/core"
import { QRCodeSVG } from "qrcode.react"
import { Button } from "@/components/ui/button"
import { Label } from "@/components/ui/label"
import { Check, Loader2 } from "lucide-react"
import type { WeChatConnection, WeChatLoginStartResult, WeChatLoginWaitResult } from "./types"

export default function WeChatConnectSection({
  accountId,
  connection,
  onConnectionChange,
}: {
  accountId?: string
  connection: WeChatConnection | null
  onConnectionChange: (connection: WeChatConnection | null) => void
}) {
  const { t } = useTranslation()
  const [sessionKey, setSessionKey] = useState<string | null>(null)
  const [qrCodeUrl, setQrCodeUrl] = useState<string | null>(null)
  const [status, setStatus] = useState<"idle" | "wait" | "scanned" | "expired" | "connected" | "error">(
    connection ? "connected" : "idle",
  )
  const [message, setMessage] = useState<string | null>(null)
  const [connecting, setConnecting] = useState(false)
  const pollingRef = useRef(false)

  useEffect(() => {
    if (connection && !sessionKey && !qrCodeUrl) {
      setStatus("connected")
      if (!message) {
        setMessage(t("channels.wechatConnected"))
      }
      return
    }

    if (!sessionKey && status === "connected") {
      setStatus("idle")
      setMessage(null)
    }
  }, [connection, message, qrCodeUrl, sessionKey, status, t])

  useEffect(() => {
    if (!sessionKey) return

    let cancelled = false

    const poll = async () => {
      if (cancelled || pollingRef.current) return
      pollingRef.current = true

      try {
        const result = await invoke<WeChatLoginWaitResult>("channel_wechat_wait_login", {
          sessionKey,
          timeoutMs: 1500,
        })

        if (cancelled) return

        if (result.connected && result.botToken && result.baseUrl) {
          onConnectionChange({
            botToken: result.botToken,
            baseUrl: result.baseUrl,
            remoteAccountId: result.remoteAccountId ?? null,
            userId: result.userId ?? null,
          })
          setStatus("connected")
          setMessage(result.message)
          setSessionKey(null)
          return
        }

        if (result.status === "scanned") {
          setStatus("scanned")
        } else if (result.status === "expired") {
          setStatus("expired")
          setSessionKey(null)
        } else {
          setStatus("wait")
        }
        setMessage(result.message)
      } catch (error) {
        if (!cancelled) {
          setStatus("error")
          setMessage(String(error))
          setSessionKey(null)
        }
      } finally {
        pollingRef.current = false
      }
    }

    void poll()
    const timer = window.setInterval(() => {
      void poll()
    }, 2000)

    return () => {
      cancelled = true
      window.clearInterval(timer)
    }
  }, [onConnectionChange, sessionKey])

  const handleStart = async () => {
    setConnecting(true)
    setMessage(null)
    setStatus("wait")

    try {
      const result = await invoke<WeChatLoginStartResult>("channel_wechat_start_login", {
        accountId: accountId ?? null,
      })
      console.log("[WeChat Login] start_login result:", {
        qrcodeUrl: result.qrcodeUrl ? `${result.qrcodeUrl.substring(0, 80)}... (${result.qrcodeUrl.length} chars)` : null,
        sessionKey: result.sessionKey,
        message: result.message,
      })
      setQrCodeUrl(result.qrcodeUrl ?? null)
      setSessionKey(result.sessionKey)
      setMessage(result.message)
    } catch (error) {
      setStatus("error")
      setMessage(String(error))
      setSessionKey(null)
    } finally {
      setConnecting(false)
    }
  }

  const identity = connection?.userId?.trim() || connection?.remoteAccountId?.trim()
  const statusText = status === "scanned"
    ? t("channels.wechatScannedHint")
    : status === "expired"
      ? t("channels.wechatExpiredHint")
      : status === "connected"
        ? t("channels.wechatConnected")
        : status === "error"
          ? message || t("common.saveFailed")
          : t("channels.wechatScanHint")

  return (
    <div className="space-y-3 rounded-lg border bg-card/60 p-4">
      <div className="flex items-start justify-between gap-3">
        <div className="space-y-1">
          <Label>{t("channels.wechatConnect")}</Label>
          <p className="text-xs text-muted-foreground">{t("channels.wechatConnectionHint")}</p>
        </div>
        <Button variant="outline" size="sm" onClick={handleStart} disabled={connecting}>
          {connecting ? <Loader2 className="mr-1 h-4 w-4 animate-spin" /> : null}
          {connection ? t("channels.wechatReconnect") : t("channels.wechatConnect")}
        </Button>
      </div>

      {connection && (
        <div className="flex items-center gap-1 text-sm text-green-600">
          <Check className="h-3.5 w-3.5" />
          {identity ? `${t("channels.wechatConnectedAs")} ${identity}` : t("channels.wechatConnected")}
        </div>
      )}

      {qrCodeUrl && status !== "connected" && (
        <div className="space-y-3">
          <div className="rounded-lg border bg-white p-3 flex justify-center">
            <QRCodeSVG value={qrCodeUrl} size={200} />
          </div>
          <div className="flex gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() => window.open(qrCodeUrl, "_blank", "noopener,noreferrer")}
            >
              {t("channels.wechatOpenQr")}
            </Button>
          </div>
        </div>
      )}

      <div className={`text-sm ${status === "error" ? "text-destructive" : "text-muted-foreground"}`}>
        {statusText}
        {message && status !== "error" ? <span className="ml-1">{message}</span> : null}
      </div>
    </div>
  )
}
