/** Extract only our bounded diagnostics, never display/log an arbitrary transport error. */
export function figmaImportError(error: unknown): {
  kind: "permission" | "rateLimited" | "other"
  retryAfterSeconds?: number
} {
  const message = error instanceof Error ? error.message : typeof error === "string" ? error : ""
  if (/Figma API returned (401|403)\b/.test(message)) return { kind: "permission" }
  if (/Figma API returned 429\b/.test(message)) {
    const match = message.match(
      /Figma API returned 429 \(rate limited(?:; retry_after_seconds=([0-9]{1,7}))?(?:; rate_limit_type=(?:low|high))?\)/,
    )
    const seconds = match?.[1] ? Number(match[1]) : undefined
    return {
      kind: "rateLimited",
      retryAfterSeconds: seconds && seconds <= 2_678_400 ? seconds : undefined,
    }
  }
  return { kind: "other" }
}
