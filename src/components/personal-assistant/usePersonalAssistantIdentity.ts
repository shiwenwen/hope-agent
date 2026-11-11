import { useCallback, useState } from "react"
import { getTransportRevision, useTransportRevision } from "@/lib/transport-provider"
import type { AssistantProfile } from "./usePersonalAssistant"

/** The sidebar identity is a projection owned by one transport connection. */
export function usePersonalAssistantIdentity() {
  const revision = useTransportRevision()
  const [identity, setIdentity] = useState<{
    profile: AssistantProfile | null
    scope: number
  } | null>(null)
  const onProfileChange = useCallback(
    (profile: AssistantProfile | null) => {
      // An earlier render's effect can flush after the connection has changed.
      if (revision !== getTransportRevision()) return
      setIdentity({ profile, scope: revision })
    },
    [revision],
  )
  return {
    profile: identity?.scope === revision ? identity.profile : null,
    onProfileChange,
  }
}
