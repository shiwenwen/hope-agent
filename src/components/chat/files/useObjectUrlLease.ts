import { useEffect, useMemo } from "react"

/** Owns exactly one object URL and revokes it on replacement/unmount. */
export function useObjectUrlLease(blob: Blob | null): string | null {
  const url = useMemo(() => (blob ? URL.createObjectURL(blob) : null), [blob])
  useEffect(
    () => () => {
      if (url) URL.revokeObjectURL(url)
    },
    [url],
  )
  return url
}
