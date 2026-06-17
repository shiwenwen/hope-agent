import { createContext } from "react"

/**
 * When true, inline tool media (images / file cards rendered by
 * {@link ToolMediaPreview}) is suppressed because an ancestor has hoisted it
 * out — e.g. {@link ProcessedBlockGroup} collects all media from the steps it
 * collapses and renders it once below the group so deliverables stay visible
 * even while the steps are folded. Default `false`: outside a hoisting group,
 * media renders inline as usual.
 */
export const MediaHoistContext = createContext(false)
