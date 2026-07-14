import { useCallback, useEffect, useState } from "react"

import {
  listenEnhancedFocusIndicators,
  loadEnhancedFocusIndicators,
  saveEnhancedFocusIndicators,
} from "@/lib/focus-indicator-preference"

export function useEnhancedFocusIndicators() {
  const [enabled, setEnabledState] = useState(
    () => document.documentElement.dataset.focusIndicators === "enhanced",
  )

  useEffect(() => {
    void loadEnhancedFocusIndicators().then(setEnabledState)
    return listenEnhancedFocusIndicators(setEnabledState)
  }, [])

  const setEnabled = useCallback((next: boolean) => {
    setEnabledState(next)
    void saveEnhancedFocusIndicators(next).catch(() => {
      void loadEnhancedFocusIndicators().then(setEnabledState)
    })
  }, [])

  return { enabled, setEnabled }
}
