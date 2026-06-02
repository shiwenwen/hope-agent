import { createContext, useContext } from "react"
import type { PreviewTarget } from "./useFilePreview"

/**
 * Ambient file-operation wiring for the message tree, so leaf components
 * (Markdown links, file cards, attachments) don't prop-drill `sessionId` +
 * `onPreviewFile` through five levels. Components rendered outside a provider
 * (Markdown in the plan panel, an extracted office preview, …) read the
 * defaults — preview is then unavailable and clicks fall back to open/download,
 * which only need the transport.
 *
 * The provider is the raw `FileActionsContext.Provider` (ChatScreen supplies a
 * memoized value); kept in a `.ts` so this module exports no component and stays
 * fast-refresh clean.
 */
export interface FileActionsContextValue {
  /** Session id used to authorize path/media reads (HTTP mode). */
  sessionId?: string | null
  /** Open the right-side preview panel; absent → preview disabled. */
  onPreviewFile?: (target: PreviewTarget) => void
}

export const FileActionsContext = createContext<FileActionsContextValue>({})

export function useFileActionsContext(): FileActionsContextValue {
  return useContext(FileActionsContext)
}
