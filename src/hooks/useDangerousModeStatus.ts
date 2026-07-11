import { useEffect, useState } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"

export interface DangerousModeStatus {
  cliFlag: boolean
  configFlag: boolean
  active: boolean
}

const DEFAULT_STATUS: DangerousModeStatus = {
  cliFlag: false,
  configFlag: false,
  active: false,
}

function normalizeStatus(value: unknown): DangerousModeStatus {
  if (typeof value !== "object" || value === null) return DEFAULT_STATUS
  const record = value as Partial<DangerousModeStatus>
  return {
    cliFlag: record.cliFlag === true,
    configFlag: record.configFlag === true,
    active: record.active === true,
  }
}

async function fetchStatus(): Promise<DangerousModeStatus> {
  const status = await getTransport().call<unknown>("get_dangerous_mode_status")
  return normalizeStatus(status)
}

/**
 * Subscribes to the global Dangerous Mode (skip-all-approvals) status.
 *
 * Source of truth lives in the backend: CLI flag (process-scoped, not
 * persisted) OR `AppConfig.dangerousSkipAllApprovals` (persisted). This
 * hook reads both via `get_dangerous_mode_status` on mount and refreshes
 * whenever `config:changed` fires on the event bus.
 */
export function useDangerousModeStatus(): DangerousModeStatus {
  const [status, setStatus] = useState<DangerousModeStatus>(DEFAULT_STATUS)

  useEffect(() => {
    let cancelled = false

    fetchStatus()
      .then((s) => {
        if (!cancelled) setStatus(s)
      })
      .catch((e) => {
        logger.error("settings", "useDangerousModeStatus::load", "Failed to load status", e)
      })

    const unlisten = getTransport().listen("config:changed", () => {
      fetchStatus()
        .then((s) => {
          if (!cancelled) setStatus(s)
        })
        .catch(() => {})
    })

    return () => {
      cancelled = true
      unlisten()
    }
  }, [])

  return status
}
