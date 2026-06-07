import type { SessionMode } from "@/types/chat"

export const SESSION_PERMISSION_MODE_ORDER: ReadonlyArray<SessionMode> = [
  "default",
  "smart",
  "yolo",
]

export function getNextPermissionMode(mode: SessionMode): SessionMode {
  const currentIndex = SESSION_PERMISSION_MODE_ORDER.indexOf(mode)
  const nextIndex = (currentIndex + 1) % SESSION_PERMISSION_MODE_ORDER.length
  return SESSION_PERMISSION_MODE_ORDER[nextIndex]
}
