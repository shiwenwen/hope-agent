/**
 * Shared directory-picker choreography for both Tauri (native dialog) and
 * HTTP (server-side `ServerDirectoryBrowser`) modes. Used by the chat-input
 * `WorkingDirectoryButton` and the `ProjectDialog` working-dir field.
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
  /** Translated toast title used when the native picker rejects the path. */
  errorTitle: string
  /** `logger.error` source label, e.g. `"ProjectDialog::pickWorkingDir"`. */
  loggerSource: string
}

export function useDirectoryPicker({
  onPicked,
  errorTitle,
  loggerSource,
}: UseDirectoryPickerOptions) {
  const [browserOpen, setBrowserOpen] = useState(false)

  const pick = useCallback(async () => {
    if (isTauriMode()) {
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
  }, [onPicked, errorTitle, loggerSource])

  const handleBrowserSelect = useCallback(
    (path: string) => {
      setBrowserOpen(false)
      onPicked(path)
    },
    [onPicked],
  )

  return { pick, browserOpen, setBrowserOpen, handleBrowserSelect }
}
