import { useSyncExternalStore } from "react"
import {
  subscribeUpdateStore,
  getPendingUpdate,
  hasChecked,
  type DesktopUpdate,
} from "@/lib/desktopUpdater"

/**
 * React hook that reactively tracks the global desktop update state.
 * Re-renders when the pending update changes.
 */
export function useDesktopUpdateStore(): {
  pendingUpdate: DesktopUpdate | null
  checked: boolean
} {
  const pendingUpdate = useSyncExternalStore(subscribeUpdateStore, getPendingUpdate)
  const checked = useSyncExternalStore(subscribeUpdateStore, hasChecked)
  return { pendingUpdate, checked }
}
