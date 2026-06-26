import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { getTransport } from "@/lib/transport-provider"
import { ExternalLink, Loader2, RefreshCw } from "lucide-react"
import { dockerInstallOptions, type DockerStatus } from "./dockerSetup"

export function DockerSetupHint({
  status,
  checking = false,
  onRefresh,
  title,
  className = "",
}: {
  status: DockerStatus | null
  checking?: boolean
  onRefresh?: () => void
  title?: string
  className?: string
}) {
  const { t } = useTranslation()

  if (checking && !status) {
    return (
      <div className={`rounded-md border border-border/50 p-3 ${className}`}>
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <Loader2 className="h-3.5 w-3.5 animate-spin" />
          {t("settings.sandboxDockerChecking")}
        </div>
      </div>
    )
  }
  if (!status || (status.installed && status.running)) return null

  const openExt = (url: string) => getTransport().call("open_url", { url })
  const options = dockerInstallOptions(status.hostOs)

  if (!status.installed) {
    return (
      <div className={`rounded-md border border-border/50 p-3 space-y-2 ${className}`}>
        <div className="text-xs font-medium">
          {title ?? t("settings.sandboxDockerUnavailable")}
        </div>
        <p className="text-xs text-muted-foreground">
          {t("settings.dockerSetupNotInstalled", {
            defaultValue: "Docker was not detected. Choose a Docker option for this platform.",
          })}
        </p>
        <Button
          size="sm"
          variant="outline"
          className="h-7 text-xs"
          onClick={() => openExt(options.primary.url)}
        >
          <ExternalLink className="h-3 w-3 mr-1" />
          {options.primary.label}
        </Button>
        <div className="text-[11px] text-muted-foreground leading-relaxed pt-0.5">
          {t("settings.webSearchDockerAlternatives")}{" "}
          {options.alternatives.map((item, idx) => (
            <span key={item.label}>
              {idx > 0 && <span className="mx-1 opacity-60">·</span>}
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="inline h-auto rounded-none px-0 py-0 text-[11px] font-normal align-baseline underline decoration-dotted underline-offset-2 hover:bg-transparent hover:text-primary"
                onClick={() => openExt(item.url)}
              >
                {item.label}
              </Button>
            </span>
          ))}
        </div>
      </div>
    )
  }

  return (
    <div className={`rounded-md border border-border/50 p-3 space-y-2 ${className}`}>
      <div className="text-xs font-medium">{title ?? t("settings.sandboxDockerUnavailable")}</div>
      <p className="text-xs text-muted-foreground">
        {t("settings.dockerSetupNotRunning", {
          defaultValue: "Docker is installed but the daemon is not running. Start Docker and try again.",
        })}
      </p>
      {onRefresh && (
        <Button size="sm" variant="outline" className="h-7 text-xs" onClick={onRefresh}>
          <RefreshCw className="h-3 w-3 mr-1" />
          {t("settings.sandboxDockerRefresh")}
        </Button>
      )}
    </div>
  )
}
