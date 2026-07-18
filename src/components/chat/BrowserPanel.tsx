import { useTranslation } from "react-i18next"
import { RightPanelShell } from "./right-panel/RightPanelShell"
import { BrowserPanelContent } from "./BrowserPanelContent"

interface BrowserPanelProps {
  sessionId?: string | null
  /** Right-panel width in px. Driven by the same drag handler ChatScreen uses
   *  for the sibling Plan / Diff / Canvas panels. */
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

/** Docked container: RightPanelShell + shared BrowserPanelContent. The live
 *  frame state lives in `useBrowserFrame`'s store so floating↔docked swaps
 *  never drop the mirror. */
export default function BrowserPanel({
  sessionId,
  panelWidth = 480,
  onPanelWidthChange,
  reservedMainWidth,
  collapsed = false,
  overlay = false,
  animateOnMount = false,
  onClose,
  onFloat,
}: BrowserPanelProps) {
  const { t } = useTranslation()
  return (
    <RightPanelShell
      width={panelWidth}
      onWidthChange={onPanelWidthChange}
      resizeLabel={t("chat.browserPanel.resizePanel", "Resize browser panel")}
      reservedMainWidth={reservedMainWidth}
      collapsed={collapsed}
      overlay={overlay}
      animateOnMount={animateOnMount}
      contentKey="browser"
    >
      <BrowserPanelContent
        variant="docked"
        sessionId={sessionId}
        active={!collapsed}
        onClose={onClose}
        onFloat={onFloat}
      />
    </RightPanelShell>
  )
}
