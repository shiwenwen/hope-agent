import { useCallback, useLayoutEffect, useRef, type Dispatch, type SetStateAction } from "react"

const DRAFT_SCOPE = "__draft__"

/** The shared jobs panel follows the active main/side conversation, not its owner. */
export function useBackgroundJobsPanelScope({
  sessionId,
  incognito,
  visible,
  setVisible,
}: {
  sessionId: string | null
  incognito: boolean
  visible: boolean
  setVisible: Dispatch<SetStateAction<boolean>>
}) {
  const dismissedRef = useRef(false)
  const suppressNextActivationRef = useRef(false)
  const previousRunningCountRef = useRef(0)
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
    suppressNextActivationRef.current = false
    previousRunningCountRef.current = 0
    setVisible(restore?.visible ?? false)
  }, [incognito, sessionId, setVisible, visible])

  const promote = useCallback((sessionId: string) => {
    if (activeScopeRef.current.key !== DRAFT_SCOPE) return
    activeScopeRef.current = { ...activeScopeRef.current, key: sessionId }
    const draft = cacheRef.current.get(DRAFT_SCOPE)
    cacheRef.current.delete(DRAFT_SCOPE)
    if (draft) cacheRef.current.set(sessionId, draft)
  }, [])

  return {
    dismissedRef,
    suppressNextActivationRef,
    previousRunningCountRef,
    promote,
  }
}
