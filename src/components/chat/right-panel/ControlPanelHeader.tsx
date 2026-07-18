import type { ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { PictureInPicture2, RefreshCw, X } from "lucide-react"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"

interface ControlPanelHeaderProps {
  icon: ReactNode
  title: string
  badge?: ReactNode
  onFloat?: () => void
  onRefresh: () => void
  onClose: () => void
}

/** Shared docked-panel header for the browser / mac control panels. */
export function ControlPanelHeader({
  icon,
  title,
  badge,
  onFloat,
  onRefresh,
  onClose,
}: ControlPanelHeaderProps) {
  const { t } = useTranslation()
  return (
    <div className="flex items-center gap-2 px-3 py-2">
      {icon}
      <div className="flex-1 truncate text-sm font-medium">{title}</div>
      {badge}
      {onFloat && (
        <IconTip label={t("chat.controlPanel.floatWindow")}>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-7 w-7 p-0"
            onClick={onFloat}
          >
            <PictureInPicture2 className="h-3.5 w-3.5" />
          </Button>
        </IconTip>
      )}
      <IconTip label={t("chat.browserPanel.refresh")}>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="h-7 w-7 p-0"
          onClick={onRefresh}
        >
          <RefreshCw className="h-3.5 w-3.5" />
        </Button>
      </IconTip>
      <IconTip label={t("chat.browserPanel.close")}>
        <Button type="button" variant="ghost" size="sm" className="h-7 w-7 p-0" onClick={onClose}>
          <X className="h-3.5 w-3.5" />
        </Button>
      </IconTip>
    </div>
  )
}
