/**
 * Shared directory-picker choreography: native on desktop by default, with
 * an app-browser opt-in for selected entry points; HTTP always uses the app
 * browser to select a directory on the machine running the agent.
 *
 * Caller still owns rendering — including mounting `<ServerDirectoryBrowser>`
 * — so per-call-site UI variations stay explicit.
 */
import { useCallback, useState } from "react"
import { toast } from "sonner"
import { isTauriMode } from "@/lib/transport"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"

interface UseDirectoryPickerOptions {
  onPicked: (path: string) => void
  /** Opt in to the app browser even when a native picker is available. */
  preferDirectoryBrowser?: boolean
  /** Translated toast title used when the native picker rejects the path. */
  errorTitle: string
  /** `logger.error` source label, e.g. `"ProjectDialog::pickWorkingDir"`. */
  loggerSource: string
}

export function useDirectoryPicker({
  onPicked,
  preferDirectoryBrowser = false,
  errorTitle,
  loggerSource,
}: UseDirectoryPickerOptions) {
  const [browserOpen, setBrowserOpen] = useState(false)
  const useDirectoryBrowser = preferDirectoryBrowser || !isTauriMode()

  const pick = useCallback(async () => {
    if (!useDirectoryBrowser) {
      try {
        const picked = await getTransport().pickLocalDirectory()
        if (picked) onPicked(picked)
      } catch (e) {
        logger.error("ui", loggerSource, "native directory picker failed", e)
        toast.error(errorTitle, {
          description: e instanceof Error ? e.message : String(e),
        })
      }
    } else {
      setBrowserOpen(true)
    }
  }, [onPicked, errorTitle, loggerSource, useDirectoryBrowser])

  const handleBrowserSelect = useCallback(
    (path: string) => {
      setBrowserOpen(false)
      onPicked(path)
    },
    [onPicked],
  )

  return { pick, browserOpen, setBrowserOpen, handleBrowserSelect, useDirectoryBrowser }
}
