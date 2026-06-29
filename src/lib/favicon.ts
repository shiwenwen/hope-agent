export function faviconPageUrlForHref(href: string | undefined): string | null {
  if (!href || href === "streamdown:incomplete-link") return null
  try {
    const url = new URL(href)
    if (url.protocol !== "http:" && url.protocol !== "https:") return null
    return `${url.origin}/`
  } catch {
    return null
  }
}

export interface SafeFaviconData {
  dataUrl: string
  mimeType: string
  sourceUrl: string
}
