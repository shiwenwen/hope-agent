import { useTranslation } from "react-i18next"
import { RightPanelShell } from "./right-panel/RightPanelShell"
import { MacControlPanelContent } from "./MacControlPanelContent"

interface MacControlPanelProps {
  sessionId?: string | null
  panelWidth?: number
  onPanelWidthChange?: (width: number) => void
  reservedMainWidth?: number
  collapsed?: boolean
  overlay?: boolean
  animateOnMount?: boolean
  onClose: () => void
  /** Switch to the in-app floating window. */
  onFloat?: () => void
}

/** Docked container: RightPanelShell + shared MacControlPanelContent. */
export default function MacControlPanel({
  sessionId,
  panelWidth = 480,
  onPanelWidthChange,
  reservedMainWidth,
  collapsed = false,
  overlay = false,
  animateOnMount = false,
  onClose,
  onFloat,
}: MacControlPanelProps) {
  const { t } = useTranslation()
  return (
    <RightPanelShell
      width={panelWidth}
      onWidthChange={onPanelWidthChange}
      resizeLabel={t("chat.browserPanel.resizePanel", "Resize panel")}
      reservedMainWidth={reservedMainWidth}
      collapsed={collapsed}
      overlay={overlay}
      animateOnMount={animateOnMount}
      contentKey="mac-control"
    >
      <MacControlPanelContent
        variant="docked"
        sessionId={sessionId}
        active={!collapsed}
        onClose={onClose}
        onFloat={onFloat}
      />
    </RightPanelShell>
  )
}
