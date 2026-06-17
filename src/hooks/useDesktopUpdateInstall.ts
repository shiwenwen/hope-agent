import { useCallback, useEffect, useRef, useState } from "react"
import {
  getDownloadStatus,
  installUpdate,
  relaunchDesktopApp,
  setPendingUpdate as setGlobalPendingUpdate,
  type DesktopUpdate,
  type DesktopUpdateEvent,
} from "@/lib/desktopUpdater"

export interface UpdateInstallController {
  /** Download + install in progress (buttons should disable + show spinner). */
  installing: boolean
  /** 0–100 while downloading, null otherwise. */
  downloadPercent: number | null
  /** Installed via "update only" and waiting for an explicit restart. */
  awaitingRestart: boolean
  /** Download (if needed) + install. `relaunchAfter` ⇒ relaunch immediately. */
  install: (relaunchAfter: boolean) => Promise<void>
  /** Relaunch now (the "restart now" affordance in the awaiting-restart state). */
  restartNow: () => Promise<void>
}

/**
 * Single owner of the desktop update install + restart lifecycle, shared by the
 * App.tsx toast and the AboutPanel settings surface so they can't drift.
 *
 * Fixes baked in:
 * - `installing` is always cleared in `finally`, so a FAILED "update only"
 *   never leaves the buttons stuck spinning.
 * - `awaitingRestart` resets whenever the target update version changes, so a
 *   stale "ready / restart" state can't leak onto a newly-discovered version
 *   that was never installed.
 */
export function useDesktopUpdateInstall(
  update: DesktopUpdate | null,
  opts?: {
    onError?: (err: unknown) => void
    beforeRelaunch?: () => void | Promise<void>
  },
): UpdateInstallController {
  const [installing, setInstalling] = useState(false)
  const [downloadPercent, setDownloadPercent] = useState<number | null>(null)
  const [awaitingRestart, setAwaitingRestart] = useState(false)

  // Reset the lifecycle when the target version changes.
  const versionRef = useRef<string | undefined>(undefined)
  useEffect(() => {
    if (update?.version !== versionRef.current) {
      versionRef.current = update?.version
      setAwaitingRestart(false)
      setInstalling(false)
      setDownloadPercent(null)
    }
  }, [update?.version])

  // Keep callbacks current without re-creating `install` each render.
  const optsRef = useRef(opts)
  optsRef.current = opts

  const install = useCallback(
    async (relaunchAfter: boolean) => {
      if (!update) return
      setInstalling(true)
      setDownloadPercent(getDownloadStatus() === "downloaded" ? 100 : 0)
      let downloaded = 0
      let contentLength = 0
      try {
        await installUpdate(update, (event: DesktopUpdateEvent) => {
          switch (event.event) {
            case "Started":
              contentLength = event.data.contentLength
              setDownloadPercent(0)
              break
            case "Progress":
              downloaded += event.data.chunkLength
              if (contentLength > 0) {
                setDownloadPercent(
                  Math.min(100, Math.round((downloaded / contentLength) * 100)),
                )
              }
              break
            case "Finished":
              setDownloadPercent(100)
              break
          }
        })
        if (relaunchAfter) {
          await optsRef.current?.beforeRelaunch?.()
          await setGlobalPendingUpdate(null)
          await relaunchDesktopApp()
        } else {
          setAwaitingRestart(true)
        }
      } catch (err) {
        optsRef.current?.onError?.(err)
      } finally {
        setInstalling(false)
      }
    },
    [update],
  )

  const restartNow = useCallback(async () => {
    await setGlobalPendingUpdate(null)
    await relaunchDesktopApp()
  }, [])

  return { installing, downloadPercent, awaitingRestart, install, restartNow }
}
