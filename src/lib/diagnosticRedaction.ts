const DEFAULT_DIAGNOSTIC_MAX_CHARS = 420

export function sanitizeDiagnosticText(
  value: string,
  maxChars = DEFAULT_DIAGNOSTIC_MAX_CHARS,
): string {
  const sanitized = value
    .replace(/\b(Authorization\s*:\s*)(Bearer|Basic)\s+[^\s,;]+/gi, "$1$2 [redacted]")
    .replace(
      /([?&](?:api[_-]?key|access[_-]?token|token|auth|authorization|key|secret|password|passphrase)=)[^&#\s]+/gi,
      "$1[redacted]",
    )
    .replace(
      /\b((?:api[_-]?key|access[_-]?token|token|secret|password|passphrase)\s*[:=]\s*)[^\s,;]+/gi,
      "$1[redacted]",
    )
    .replace(/\bsk-[A-Za-z0-9_-]{8,}\b/g, "sk-[redacted]")
    .replace(/\bAIza[0-9A-Za-z_-]{20,}\b/g, "AIza[redacted]")
    .replace(/\s+/g, " ")
    .trim()
  if (sanitized.length <= maxChars) return sanitized
  return `${sanitized.slice(0, Math.max(0, maxChars - 16)).trimEnd()} ... [truncated]`
}
