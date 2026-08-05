import { useEffect, useState } from "react"
import { getTransport } from "@/lib/transport-provider"
import builtinPetUrl from "@/assets/pets/hope-default.webp"
import debugPetUrl from "@/assets/pets/hope-debug.png"
import { BUILTIN_DEBUG_PET_ASSET_ID } from "@/types/pet"

export function usePetAssetUrl(assetId: string | null): {
  src: string
  loading: boolean
  failed: boolean
  fallback: boolean
} {
  const [resolved, setResolved] = useState<{
    assetId: string
    src: string
    failed: boolean
  } | null>(null)

  useEffect(() => {
    let disposed = false
    let revoke: () => void = () => undefined
    if (!assetId || (import.meta.env.DEV && assetId === BUILTIN_DEBUG_PET_ASSET_ID)) return
    void getTransport()
      .loadPetAsset(assetId, builtinPetUrl)
      .then((lease) => {
        if (disposed) {
          lease.revoke()
          return
        }
        revoke = lease.revoke
        setResolved({ assetId, src: lease.src, failed: false })
      })
      .catch(() => {
        if (!disposed) setResolved({ assetId, src: builtinPetUrl, failed: true })
      })
    return () => {
      disposed = true
      revoke()
    }
  }, [assetId])

  if (!assetId) return { src: builtinPetUrl, loading: false, failed: false, fallback: true }
  if (import.meta.env.DEV && assetId === BUILTIN_DEBUG_PET_ASSET_ID) {
    return { src: debugPetUrl, loading: false, failed: false, fallback: false }
  }
  if (resolved?.assetId !== assetId) {
    return { src: builtinPetUrl, loading: true, failed: false, fallback: true }
  }
  return {
    src: resolved.src,
    loading: false,
    failed: resolved.failed,
    fallback: resolved.failed,
  }
}
