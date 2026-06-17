import { useSyncExternalStore } from "react"
import {
  subscribeUpdateStore,
  getPendingUpdate,
  hasChecked,
  getDownloadStatus,
  type DesktopUpdate,
  type DownloadStatus,
} from "@/lib/desktopUpdater"

/**
 * React hook that reactively tracks the global desktop update state.
 * Re-renders when the pending update or its download status changes.
 */
export function useDesktopUpdateStore(): {
  pendingUpdate: DesktopUpdate | null
  checked: boolean
  downloadStatus: DownloadStatus
} {
  const pendingUpdate = useSyncExternalStore(subscribeUpdateStore, getPendingUpdate)
  const checked = useSyncExternalStore(subscribeUpdateStore, hasChecked)
  const downloadStatus = useSyncExternalStore(subscribeUpdateStore, getDownloadStatus)
  return { pendingUpdate, checked, downloadStatus }
}
