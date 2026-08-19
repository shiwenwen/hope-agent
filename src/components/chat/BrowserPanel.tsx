import { RightPanelShell } from "./right-panel/RightPanelShell"
import { BrowserPanelContent } from "./BrowserPanelContent"

interface BrowserPanelProps {
  sessionId?: string | null
  collapsed?: boolean
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
  collapsed = false,
  animateOnMount = false,
  onClose,
  onFloat,
}: BrowserPanelProps) {
  return (
    <RightPanelShell
      collapsed={collapsed}
      animateOnMount={animateOnMount}
      contentKey="browser"
    >
      <BrowserPanelContent
        variant="docked"
        sessionId={sessionId}
        active={!collapsed}
        onClose={onClose}
        onFloat={onFloat}
        integrated
      />
    </RightPanelShell>
  )
}
