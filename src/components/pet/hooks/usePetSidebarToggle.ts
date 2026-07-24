import { useCallback, useEffect, useRef, useState } from "react"
import { isTauriMode } from "@/lib/transport"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { PetConfig } from "@/types/pet"

export function usePetSidebarToggle() {
  const supported = isTauriMode()
  const [enabled, setEnabled] = useState<boolean | null>(null)
  const [updating, setUpdating] = useState(false)
  const inFlight = useRef(false)

  const reload = useCallback(async () => {
    if (!supported) return
    try {
      const config = await getTransport().call<PetConfig>("get_pet_config_cmd")
      setEnabled(config.enabled)
    } catch (error) {
      logger.warn("pet", "sidebar_load", "Failed to load the Pet sidebar switch", error)
    }
  }, [supported])

  useEffect(() => {
    if (!supported) return
    void reload()
    return getTransport().listen("pet:config_changed", () => void reload())
  }, [reload, supported])

  const toggle = useCallback(async (): Promise<boolean> => {
    if (enabled === null || inFlight.current) return false
    const previous = enabled
    const next = !previous
    inFlight.current = true
    setUpdating(true)
    setEnabled(next)
    try {
      const config = await getTransport().call<PetConfig>("pet_set_enabled_cmd", {
        enabled: next,
        source: "sidebar",
      })
      setEnabled(config.enabled)
      return true
    } catch (error) {
      setEnabled(previous)
      logger.warn("pet", "sidebar_toggle", "Failed to toggle the Pet from the sidebar", error)
      return false
    } finally {
      inFlight.current = false
      setUpdating(false)
    }
  }, [enabled])

  return {
    supported,
    enabled: enabled ?? false,
    ready: enabled !== null,
    updating,
    toggle,
  }
}
