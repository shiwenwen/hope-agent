import { createPortal } from "react-dom"
import { useTranslation } from "react-i18next"
import { Globe, Monitor } from "lucide-react"
import type { FloatablePanel } from "@/hooks/useFloatingPanels"
import { useBrowserFrame } from "@/hooks/useBrowserFrame"
import { useMacControlFrame } from "@/hooks/useMacControlFrame"
import { BrowserPanelContent } from "../BrowserPanelContent"
import { MacControlPanelContent } from "../MacControlPanelContent"
import { FloatingPanelWindow } from "./FloatingPanelWindow"

interface FloatingPanelLayerProps {
  floatingPanels: FloatablePanel[]
  zIndexOf: (panel: FloatablePanel) => number
  onDock: (panel: FloatablePanel) => void
  onClose: (panel: FloatablePanel) => void
  onFocus: (panel: FloatablePanel) => void
  sessionId?: string | null
}

function FloatingBrowserWindow({
  zIndex,
  onDock,
  onClose,
  onFocus,
  sessionId,
}: {
  zIndex: number
  onDock: () => void
  onClose: () => void
  onFocus: () => void
  sessionId?: string | null
}) {
  const { t } = useTranslation()
  // Title-only subscription; the mirror itself renders inside the content.
  const { frame } = useBrowserFrame({ sessionId, pollKey: "floating-title", pollActive: false })
  return (
    <FloatingPanelWindow
      storageKey="hope.floatingPanel.browser.rect"
      icon={<Globe className="h-3.5 w-3.5 text-muted-foreground" />}
      title={frame?.title || t("chat.browserPanel.idleTitle")}
      zIndex={zIndex}
      onDock={onDock}
      onClose={onClose}
      onFocus={onFocus}
    >
      <BrowserPanelContent variant="floating" sessionId={sessionId} onClose={onClose} />
    </FloatingPanelWindow>
  )
}

function FloatingMacControlWindow({
  zIndex,
  onDock,
  onClose,
  onFocus,
  sessionId,
}: {
  zIndex: number
  onDock: () => void
  onClose: () => void
  onFocus: () => void
  sessionId?: string | null
}) {
  const { t } = useTranslation()
  const { frame } = useMacControlFrame({ pollKey: "floating-title", pollActive: false })
  return (
    <FloatingPanelWindow
      storageKey="hope.floatingPanel.macControl.rect"
      icon={<Monitor className="h-3.5 w-3.5 text-muted-foreground" />}
      title={frame?.frontmostApp?.name || t("settings.macControl.title")}
      zIndex={zIndex}
      onDock={onDock}
      onClose={onClose}
      onFocus={onFocus}
    >
      <MacControlPanelContent variant="floating" sessionId={sessionId} onClose={onClose} />
    </FloatingPanelWindow>
  )
}

/** Portal layer hosting the floating control-panel windows. */
export function FloatingPanelLayer({
  floatingPanels,
  zIndexOf,
  onDock,
  onClose,
  onFocus,
  sessionId,
}: FloatingPanelLayerProps) {
  if (floatingPanels.length === 0) return null
  return createPortal(
    <>
      {floatingPanels.includes("browser") && (
        <FloatingBrowserWindow
          zIndex={zIndexOf("browser")}
          onDock={() => onDock("browser")}
          onClose={() => onClose("browser")}
          onFocus={() => onFocus("browser")}
          sessionId={sessionId}
        />
      )}
      {floatingPanels.includes("mac-control") && (
        <FloatingMacControlWindow
          zIndex={zIndexOf("mac-control")}
          onDock={() => onDock("mac-control")}
          onClose={() => onClose("mac-control")}
          onFocus={() => onFocus("mac-control")}
          sessionId={sessionId}
        />
      )}
    </>,
    document.body,
  )
}
