export function normalizeAudioCaptureError(value: unknown): Error {
  if (value instanceof Error) return value

  if (value && typeof value === "object") {
    const record = value as { name?: unknown; message?: unknown }
    const message = typeof record.message === "string" ? record.message : String(value)
    const error = new Error(message)
    if (typeof record.name === "string" && record.name) error.name = record.name
    return error
  }

  return new Error(String(value))
}

export function isAudioCapturePermissionError(error: Error): boolean {
  return error.name === "NotAllowedError" || error.name === "SecurityError"
}
