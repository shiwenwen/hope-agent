import { useState, useEffect, useRef } from "react"
import { invoke } from "@tauri-apps/api/core"
import UrlPreviewCard, { type UrlPreviewData } from "@/components/chat/UrlPreviewCard"
import { extractUrls } from "@/lib/urlDetect"

/** Renders URL preview cards for a message's content */
export default function MessageUrlPreviews({ content, isStreaming }: { content: string; isStreaming: boolean }) {
  const [previews, setPreviews] = useState<UrlPreviewData[]>([])
  const fetchedRef = useRef(false)

  useEffect(() => {
    if (isStreaming || fetchedRef.current || !content.trim()) return

    const urls = extractUrls(content)
    if (urls.length === 0) return

    fetchedRef.current = true
    const urlsToFetch = urls.slice(0, 5)

    invoke<UrlPreviewData[]>("fetch_url_previews", { urls: urlsToFetch })
      .then(setPreviews)
      .catch(() => {})
  }, [content, isStreaming])

  if (previews.length === 0) return null

  return (
    <div className="mt-2 flex flex-col gap-1.5">
      {previews.map((p) => (
        <UrlPreviewCard key={p.url} data={p} />
      ))}
    </div>
  )
}
