import { getTransport } from "@/lib/transport-provider"

export interface DesktopOpenResult {
  ok?: boolean
}

export function openExternalUrl(url: string): void {
  const openInBrowser = () => window.open(url, "_blank", "noopener")
  void getTransport()
    .call<DesktopOpenResult | void>("open_url", { url })
    .then((result) => {
      if (result && typeof result === "object" && result.ok === false) {
        openInBrowser()
      }
    })
    .catch(openInBrowser)
}
