import { useMemoryData } from "./useMemoryData"
import EmbeddingView from "./EmbeddingView"
import MemoryFormView from "./MemoryFormView"
import MemoryListView from "./MemoryListView"

/**
 * MemoryPanel - Memory management UI.
 *
 * Two modes:
 * - **Standalone** (no agentId): Global view with agent scope filter dropdown.
 *   Used in Settings > Memory tab.
 * - **Embedded** (agentId provided): Agent-scoped view showing only that agent's
 *   memories + global memories. Used inside Agent edit panel's Memory tab.
 */
export default function MemoryPanel({ agentId, compact }: { agentId?: string; compact?: boolean }) {
  const isAgentMode = !!agentId

  const data = useMemoryData({ agentId, isAgentMode })

  // ── Embedding Config View ──
  if (data.view === "embedding") {
    return <EmbeddingView data={data} />
  }

  // ── Add / Edit View ──
  if (data.view === "add" || data.view === "edit") {
    return <MemoryFormView data={data} />
  }

  // ── List View (default) ──
  return <MemoryListView data={data} isAgentMode={isAgentMode} compact={compact} />
}
