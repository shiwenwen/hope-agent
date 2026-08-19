import { RightPanelShell } from "./right-panel/RightPanelShell"
import { MacControlPanelContent } from "./MacControlPanelContent"

interface MacControlPanelProps {
  sessionId?: string | null
  collapsed?: boolean
  animateOnMount?: boolean
  onClose: () => void
  /** Switch to the in-app floating window. */
  onFloat?: () => void
}

/** Docked container: RightPanelShell + shared MacControlPanelContent. */
export default function MacControlPanel({
  sessionId,
  collapsed = false,
  animateOnMount = false,
  onClose,
  onFloat,
}: MacControlPanelProps) {
  return (
    <RightPanelShell
      collapsed={collapsed}
      animateOnMount={animateOnMount}
      contentKey="mac-control"
    >
      <MacControlPanelContent
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
