import type { MentionSessionCandidate } from "../mentions/typedMentions"

/** Keep the visible title inside a Markdown link label. A backslash can escape
 * the closing bracket, so it must be normalized alongside brackets/newlines. */
export function safeSessionMentionLabel(value: string): string {
  const cleaned = value
    .replaceAll("\\", " ")
    .replaceAll("[", " ")
    .replaceAll("]", " ")
    .replaceAll("\n", " ")
    .replaceAll("\r", " ")
    .trim()
  // The typed-wire label budget is 256 UTF-8 bytes. Sixty Unicode scalars
  // stay within 240 bytes even when every character is four-byte UTF-8.
  return Array.from(cleaned).slice(0, 60).join("")
}

export function sessionDisplayLabel(candidate: MentionSessionCandidate): string {
  return safeSessionMentionLabel(candidate.title) || candidate.id.slice(0, 8)
}

export function formatSessionInsertion(candidate: MentionSessionCandidate): string {
  return `[@${sessionDisplayLabel(candidate)}](#session:${candidate.id})`
}
