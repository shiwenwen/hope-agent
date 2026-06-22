import { useTranslation } from "react-i18next"
import IconChrome from "~icons/tabler/brand-chrome"
import { Button } from "@/components/ui/button"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import { useBrowserBackendStatus } from "@/hooks/useBrowserBackendStatus"

interface BrowserStatusIndicatorProps {
  onOpen?: () => void
  className?: string
}

/**
 * Sidebar status indicator for the browser backend — mirrors
 * [`ServerStatusIndicator`]. Green dot when a backend is live (extension
 * available or a CDP session connected), muted otherwise; hover shows details,
 * click opens Settings → Browser. Polls the two cheap in-memory status reads
 * every 5s (never the pgrep-based doctor probe).
 */
export default function BrowserStatusIndicator({ onOpen, className }: BrowserStatusIndicatorProps) {
  const { t } = useTranslation()
  const { ext, cdp } = useBrowserBackendStatus(5000)

  const extLive = ext?.backendAvailable ?? false
  const cdpLive = cdp?.connected ?? false
  const live = extLive || cdpLive

  const backendLabel = extLive
    ? t("settings.browser.method.extension")
    : cdpLive
      ? cdp?.mode === "launch"
        ? t("settings.browser.method.managed")
        : t("settings.browser.method.attach")
      : null

  // Localized "why not connected" reason, reusing the extension-kind labels.
  const kindLabel: Record<string, string> = {
    host_missing: t("settings.browser.extension.kindHostMissing"),
    broker_unavailable: t("settings.browser.extension.kindBrokerUnavailable"),
    version_mismatch: t("settings.browser.extension.kindVersionMismatch"),
    extension_missing: t("settings.browser.extension.kindExtensionMissing"),
  }
  const disconnectedReason = ext ? (kindLabel[ext.kind] ?? null) : null

  const tooltipBody = (
    <div className="text-xs space-y-1 max-w-[260px]">
      <div className="font-medium">
        {live ? t("settings.browser.statusConnected") : t("settings.browser.statusDisconnected")}
      </div>
      {live ? (
        <>
          {backendLabel && (
            <div className="text-muted-foreground">
              {t("settings.browser.methodLabel")}:{" "}
              <span className="text-foreground">{backendLabel}</span>
            </div>
          )}
          {extLive && ext?.extensionVersion && (
            <div className="text-muted-foreground">
              {t("settings.browser.extension.title")}:{" "}
              <span className="text-foreground">{ext.extensionVersion}</span>
            </div>
          )}
          {cdpLive && cdp && cdp.tabs.length > 0 && (
            <div className="text-muted-foreground">
              {t("settings.browser.tabCount", { count: cdp.tabs.length })}
            </div>
          )}
        </>
      ) : (
        <div className="text-muted-foreground">
          {disconnectedReason ?? t("settings.browser.method.extensionDesc")}
        </div>
      )}
    </div>
  )

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          onClick={onOpen}
          className={cn(
            "relative rounded-xl h-8 w-8 text-muted-foreground hover:text-foreground",
            className,
          )}
          aria-label={t("settings.browser.title")}
        >
          {/* Tabler's chrome glyph is inset more than lucide's (~75% vs ~83% of
              the viewBox), so bump it up a touch to match the rail's icon size. */}
          <IconChrome className="h-[18px] w-[18px]" />
          <span
            className={cn(
              "absolute -top-0.5 -right-0.5 h-2.5 w-2.5 rounded-full border-2 border-background",
              live ? "bg-green-500" : "bg-muted-foreground/40",
            )}
          />
        </Button>
      </TooltipTrigger>
      <TooltipContent side="right" className="p-2.5">
        {tooltipBody}
      </TooltipContent>
    </Tooltip>
  )
}
