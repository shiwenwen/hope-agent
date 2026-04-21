import { useTranslation } from "react-i18next"
import { Radio, AlertTriangle, Loader2 } from "lucide-react"
import { Button } from "@/components/ui/button"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import {
  useServerStatus,
  formatServerUptime,
  formatActiveChatCounts,
  formatActiveConnectionsSub,
  totalActiveConnections,
} from "@/hooks/useServerStatus"

interface ServerStatusIndicatorProps {
  onOpen?: () => void
  className?: string
}

export default function ServerStatusIndicator({
  onOpen,
  className,
}: ServerStatusIndicatorProps) {
  const { t } = useTranslation()
  const { status, loading, error } = useServerStatus(5000)

  const isFailed = Boolean(status?.startupError) || (!status && !loading)
  const wsTotal = status ? totalActiveConnections(status) : 0
  const wsSub = status ? formatActiveConnectionsSub(status, t) : ""
  const chatCountsSub = status
    ? formatActiveChatCounts(status.activeChatCounts, t)
    : null

  const dotColor = isFailed
    ? "bg-destructive"
    : status
      ? "bg-green-500"
      : "bg-muted-foreground/40"

  const tooltipBody = (
    <div className="text-xs space-y-1 max-w-[260px]">
      <div className="font-medium">
        {isFailed
          ? t("settings.serverStartupError")
          : t("settings.serverRuntimeStatus")}
      </div>
      {status?.startupError ? (
        <pre className="whitespace-pre-wrap break-all text-destructive opacity-90">
          {status.startupError}
        </pre>
      ) : status ? (
        <>
          <div className="text-muted-foreground">
            {t("settings.serverBoundAddr")}:{" "}
            <span className="text-foreground">
              {status.boundAddr ?? t("settings.serverNotStarted")}
            </span>
          </div>
          <div className="text-muted-foreground">
            {t("settings.serverUptime")}:{" "}
            <span className="text-foreground">
              {formatServerUptime(status.uptimeSecs)}
            </span>
          </div>
          <div className="text-muted-foreground">
            {t("settings.serverActiveWebSockets")}:{" "}
            <span className="text-foreground">{wsTotal}</span>{" "}
            <span className="opacity-70">({wsSub})</span>
          </div>
          <div className="text-muted-foreground">
            {t("settings.serverActiveChatStreams")}:{" "}
            <span className="text-foreground">
              {status.activeChatCounts.total}
            </span>
            {chatCountsSub && (
              <span className="opacity-70"> ({chatCountsSub})</span>
            )}
          </div>
        </>
      ) : (
        <div className="text-muted-foreground">
          {error ?? t("settings.serverNotStarted")}
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
          aria-label={t("settings.serverRuntimeStatus")}
        >
          {loading && !status ? (
            <Loader2 className="h-4 w-4 animate-spin" />
          ) : isFailed ? (
            <AlertTriangle className="h-4 w-4 text-destructive" />
          ) : (
            <Radio className="h-4 w-4" />
          )}
          <span
            className={cn(
              "absolute -top-0.5 -right-0.5 h-2.5 w-2.5 rounded-full border-2 border-background",
              dotColor,
            )}
          />
          {!isFailed && status && wsTotal > 0 && (
            <span className="absolute -bottom-1 -right-1 min-w-[14px] h-[14px] px-1 rounded-full bg-primary text-primary-foreground text-[9px] font-semibold flex items-center justify-center">
              {wsTotal > 99 ? "99+" : wsTotal}
            </span>
          )}
        </Button>
      </TooltipTrigger>
      <TooltipContent side="right" className="p-2.5">
        {tooltipBody}
      </TooltipContent>
    </Tooltip>
  )
}
