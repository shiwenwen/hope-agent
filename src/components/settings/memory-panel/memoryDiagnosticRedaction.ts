import { sanitizeDiagnosticText } from "../../../lib/diagnosticRedaction"

export function sanitizeMemoryDiagnosticText(
  value: string,
  maxChars?: number,
): string {
  return sanitizeDiagnosticText(value, maxChars)
}
