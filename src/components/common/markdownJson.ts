export function shouldRenderAsBareJson(content: string): boolean {
  const trimmed = content.trimStart()
  if (!trimmed) return false
  if (trimmed.startsWith("```")) return false

  const first = trimmed[0]
  if (first !== "{" && first !== "[") return false

  // Avoid JSON.parse here: during streaming the payload is incomplete for most
  // frames, and repeatedly parsing a growing tool-result-sized JSON blob is
  // exactly the kind of work that makes the text appear to repaint.
  const sample = trimmed.slice(0, 512)
  if (first === "{") {
    return (
      sample === "{" ||
      sample === "{}" ||
      sample.includes("\n") ||
      /"[^"\n]+"\s*:/.test(sample)
    )
  }
  return (
    sample === "[" ||
    sample === "[]" ||
    /^\[\s*(?:\{|\[|"|-?\d|true\b|false\b|null\b)/.test(sample)
  )
}
