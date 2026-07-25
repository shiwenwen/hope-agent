import { useCallback, useEffect, useState } from "react"

export const PET_DISCOVERY_STORAGE_KEY = "hope-agent.pet-discovery.v1"
export const PET_DISCOVERY_DELAY_MS = 700

function hasDiscoveredPet(): boolean {
  if (typeof window === "undefined") return false
  try {
    return window.localStorage.getItem(PET_DISCOVERY_STORAGE_KEY) === "seen"
  } catch {
    return false
  }
}

function persistPetDiscovery(): void {
  if (typeof window === "undefined") return
  try {
    window.localStorage.setItem(PET_DISCOVERY_STORAGE_KEY, "seen")
  } catch {
    // A cosmetic discovery hint must never block the Pet control.
  }
}

interface PetDiscoveryHintOptions {
  supported: boolean
  ready: boolean
  enabled: boolean
}

export function usePetDiscoveryHint({ supported, ready, enabled }: PetDiscoveryHintOptions) {
  const [discovered, setDiscovered] = useState(hasDiscoveredPet)
  const [snoozed, setSnoozed] = useState(false)
  const [open, setOpen] = useState(false)
  const eligible = supported && ready && !enabled && !discovered && !snoozed

  const markDiscovered = useCallback(() => {
    persistPetDiscovery()
    setDiscovered(true)
    setOpen(false)
  }, [])

  useEffect(() => {
    if (supported && ready && enabled && !discovered) markDiscovered()
  }, [discovered, enabled, markDiscovered, ready, supported])

  useEffect(() => {
    if (!eligible) {
      setOpen(false)
      return
    }
    const timer = window.setTimeout(() => setOpen(true), PET_DISCOVERY_DELAY_MS)
    return () => window.clearTimeout(timer)
  }, [eligible])

  const handleOpenChange = useCallback(
    (nextOpen: boolean) => {
      if (nextOpen && eligible) {
        setOpen(true)
        return
      }
      setOpen(false)
      if (!nextOpen) setSnoozed(true)
    },
    [eligible],
  )

  return { open, handleOpenChange, markDiscovered }
}
