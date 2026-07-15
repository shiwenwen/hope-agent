import { useFileActions, type FileActionsOverrides, type FileActionsResult } from "./useFileActions"
import type { FileTarget } from "./types"

/** Unified React entry point for primary clicks, menus, header actions and capabilities. */
export function useFileResource(
  target: FileTarget | null,
  overrides?: FileActionsOverrides,
): FileActionsResult {
  return useFileActions(target, overrides)
}
