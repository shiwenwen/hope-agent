import { useCallback, useLayoutEffect, useRef, type Dispatch, type RefObject, type SetStateAction } from "react"

const DRAFT_SCOPE = "__draft__"

interface BrowserPanelScopeOptions {
  sessionId: string | null
  incognito: boolean
  visible: boolean
  setVisible: Dispatch<SetStateAction<boolean>>
  dismissedRef: RefObject<boolean>
  closeFloating: (panel: "browser") => void
}

/** Browser frames belong to the active conversation, including embedded side chats. */
export function useBrowserPanelSessionScope({
  sessionId,
  incognito,
  visible,
  setVisible,
  dismissedRef,
  closeFloating,
}: BrowserPanelScopeOptions) {
  const cacheRef = useRef(new Map<string, { visible: boolean; dismissed: boolean }>())
  const activeScopeRef = useRef({ key: DRAFT_SCOPE, incognito: false })

  useLayoutEffect(() => {
    const key = sessionId ?? DRAFT_SCOPE
    const previous = activeScopeRef.current
    if (previous.key === key && previous.incognito === incognito) return
    if (!previous.incognito) {
      cacheRef.current.set(previous.key, { visible, dismissed: dismissedRef.current })
    }
    const restore = incognito ? undefined : cacheRef.current.get(key)
    activeScopeRef.current = { key, incognito }
    dismissedRef.current = restore?.dismissed ?? false
    setVisible(restore?.visible ?? false)
    closeFloating("browser")
  }, [closeFloating, dismissedRef, incognito, sessionId, setVisible, visible])

  return useCallback((sessionId: string) => {
    if (activeScopeRef.current.key !== DRAFT_SCOPE) return
    activeScopeRef.current = { ...activeScopeRef.current, key: sessionId }
    const draft = cacheRef.current.get(DRAFT_SCOPE)
    cacheRef.current.delete(DRAFT_SCOPE)
    if (draft) cacheRef.current.set(sessionId, draft)
  }, [])
}
