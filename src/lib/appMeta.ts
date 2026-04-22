import { startTransition, useEffect, useState } from "react"
import { isTauriMode } from "@/lib/transport"

export const APP_VERSION = typeof __APP_VERSION__ === "string" ? __APP_VERSION__ : "0.1.0"

export const HOPE_AGENT_URLS = {
  github: "https://github.com/shiwenwen/hope-agent",
  releases: "https://github.com/shiwenwen/hope-agent/releases",
  feedback: "https://github.com/shiwenwen/hope-agent/issues",
} as const

let appVersionPromise: Promise<string> | null = null

export async function getAppVersion(): Promise<string> {
  if (!isTauriMode()) return APP_VERSION

  if (!appVersionPromise) {
    appVersionPromise = import("@tauri-apps/api/app")
      .then(({ getVersion }) => getVersion())
      .catch(() => APP_VERSION)
  }

  return appVersionPromise
}

export function useAppVersion(): string {
  const [version, setVersion] = useState(APP_VERSION)

  useEffect(() => {
    let cancelled = false

    void getAppVersion().then((nextVersion) => {
      if (cancelled) return
      startTransition(() => setVersion(nextVersion))
    })

    return () => {
      cancelled = true
    }
  }, [])

  return version
}
