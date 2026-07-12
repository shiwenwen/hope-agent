import { getTransport } from "@/lib/transport-provider"

export interface DesktopOpenResult {
  ok?: boolean
}

export interface OpenExternalUrlOptions {
  onError?: (error: unknown) => void
}

export function openExternalUrl(url: string, options: OpenExternalUrlOptions = {}): void {
  const openInBrowser = () => {
    const opened = window.open(url, "_blank", "noopener")
    if (!opened) throw new Error("Browser blocked opening the link")
  }
  void (async () => {
    try {
      const result = await getTransport().call<DesktopOpenResult | void>("open_url", { url })
      if (!result || typeof result !== "object" || result.ok !== false) return
    } catch {
      // Fall back to the browser if the owner command is unavailable.
    }
    try {
      openInBrowser()
    } catch (e) {
      options.onError?.(e)
    }
  })()
}
