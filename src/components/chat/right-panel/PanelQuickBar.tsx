import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { ArrowLeft, ArrowRight, Camera, ExternalLink, Hand, RotateCw } from "lucide-react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { IconTip } from "@/components/ui/tooltip"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import type { MacControlDisplaysResponse } from "@/hooks/useMacControlFrame"

// ── Browser quick bar ────────────────────────────────────────────────────

interface BrowserQuickBarProps {
  sessionId?: string | null
  currentUrl?: string | null
  paused: boolean
  onTogglePaused: () => void
}

/** URL bar + back/reload + take-over/open-external for the browser panel. */
export function BrowserQuickBar({
  sessionId,
  currentUrl,
  paused,
  onTogglePaused,
}: BrowserQuickBarProps) {
  const { t } = useTranslation()
  const [url, setUrl] = useState("")
  const [busy, setBusy] = useState(false)

  const navigate = async (op: "go" | "back" | "reload") => {
    if (busy) return
    setBusy(true)
    try {
      await getTransport().call("browser_panel_navigate", {
        op,
        url: op === "go" ? url.trim() : undefined,
        sessionId: sessionId ?? undefined,
      })
      if (op === "go") setUrl("")
    } catch (e) {
      logger.warn("ui", "PanelQuickBar::navigate", `browser_panel_navigate ${op} failed`, e)
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="flex items-center gap-1 border-t border-border/60 px-2 py-1.5">
      <IconTip label={t("chat.controlPanel.quick.back")}>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="h-7 w-7 shrink-0 p-0"
          disabled={busy}
          onClick={() => void navigate("back")}
        >
          <ArrowLeft className="h-3.5 w-3.5" />
        </Button>
      </IconTip>
      <IconTip label={t("chat.controlPanel.quick.reload")}>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="h-7 w-7 shrink-0 p-0"
          disabled={busy}
          onClick={() => void navigate("reload")}
        >
          <RotateCw className="h-3.5 w-3.5" />
        </Button>
      </IconTip>
      <form
        className="flex min-w-0 flex-1 items-center gap-1"
        onSubmit={(e) => {
          e.preventDefault()
          if (url.trim()) void navigate("go")
        }}
      >
        <Input
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          placeholder={t("chat.controlPanel.quick.urlPlaceholder")}
          className="h-7 min-w-0 flex-1 text-xs"
        />
        <IconTip label={t("chat.controlPanel.quick.go")}>
          <Button
            type="submit"
            variant="ghost"
            size="sm"
            className="h-7 w-7 shrink-0 p-0"
            disabled={busy || !url.trim()}
          >
            <ArrowRight className="h-3.5 w-3.5" />
          </Button>
        </IconTip>
      </form>
      <IconTip label={t("chat.browserPanel.takeOverHint")}>
        <Button
          type="button"
          variant={paused ? "default" : "ghost"}
          size="sm"
          className="h-7 w-7 shrink-0 p-0"
          onClick={onTogglePaused}
        >
          <Hand className="h-3.5 w-3.5" />
        </Button>
      </IconTip>
      {currentUrl && (
        <IconTip label={t("chat.browserPanel.openExternal")}>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-7 w-7 shrink-0 p-0"
            onClick={() => window.open(currentUrl, "_blank")}
          >
            <ExternalLink className="h-3.5 w-3.5" />
          </Button>
        </IconTip>
      )}
    </div>
  )
}

// ── Mac control quick bar ────────────────────────────────────────────────

interface MacQuickBarProps {
  displayId: number | null
  onDisplayChange: (displayId: number | null) => void
  onCaptureNow: () => void
}

/** Display-target picker + capture-now for the mac control panel. */
export function MacQuickBar({ displayId, onDisplayChange, onCaptureNow }: MacQuickBarProps) {
  const { t } = useTranslation()
  const [displays, setDisplays] = useState<MacControlDisplaysResponse["displays"]>([])

  useEffect(() => {
    let alive = true
    getTransport()
      .call<MacControlDisplaysResponse>("mac_control_list_displays")
      .then((response) => {
        if (alive && !response.error) setDisplays(response.displays ?? [])
      })
      .catch((e) => {
        logger.warn("ui", "PanelQuickBar::displays", "mac_control_list_displays failed", e)
      })
    return () => {
      alive = false
    }
  }, [])

  return (
    <div className="flex items-center gap-1.5 border-t border-border/60 px-2 py-1.5">
      {displays.length > 1 && (
        <Select
          value={displayId != null ? String(displayId) : "main"}
          onValueChange={(value) => onDisplayChange(value === "main" ? null : Number(value))}
        >
          <SelectTrigger className="h-7 w-auto min-w-32 text-xs">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="main">{t("chat.controlPanel.quick.mainDisplay")}</SelectItem>
            {displays.map((display, index) => (
              <SelectItem key={display.id} value={String(display.id)}>
                {t("chat.controlPanel.quick.displayN", { n: index + 1 })}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      )}
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="h-7 gap-1.5 px-2 text-xs"
        onClick={onCaptureNow}
      >
        <Camera className="h-3.5 w-3.5" />
        {t("chat.controlPanel.quick.captureNow")}
      </Button>
    </div>
  )
}
