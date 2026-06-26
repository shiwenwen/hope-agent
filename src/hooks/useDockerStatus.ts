import { useCallback, useState } from "react"
import { getTransport } from "@/lib/transport-provider"
import type { DockerStatus } from "@/components/settings/dockerSetup"

/**
 * Shared Docker-availability probe used by the sandbox UIs (chat sandbox
 * switcher, agent capabilities tab, cron job form). Wraps the
 * `check_sandbox_available` transport call with status/checking state and a
 * derived `ready` flag, so every sandbox surface gates on the same source of
 * truth instead of re-implementing the fetch.
 */
export function useDockerStatus() {
  const [status, setStatus] = useState<DockerStatus | null>(null)
  const [checking, setChecking] = useState(false)

  const refresh = useCallback(async () => {
    setChecking(true)
    try {
      setStatus(await getTransport().call<DockerStatus>("check_sandbox_available"))
    } catch {
      // Best-effort hint; the backend remains the real gate at run time.
    } finally {
      setChecking(false)
    }
  }, [])

  const ready = !!(status?.installed && status?.running)
  return { status, checking, ready, refresh }
}
