import { useEffect, useState } from "react"
import AgentListView from "./AgentListView"
import AgentEditView from "./AgentEditView"
import type { AgentTab } from "./types"

export default function AgentPanel({
  initialAgentId,
  initialAgentTab,
}: {
  initialAgentId?: string
  initialAgentTab?: AgentTab
}) {
  const [editingId, setEditingId] = useState<string | null>(initialAgentId ?? null)
  const [pendingInitialTab, setPendingInitialTab] = useState<AgentTab | undefined>(
    initialAgentTab,
  )

  useEffect(() => {
    if (!initialAgentId) return
    let cancelled = false
    queueMicrotask(() => {
      if (cancelled) return
      setEditingId(initialAgentId)
      setPendingInitialTab(initialAgentTab)
    })
    return () => {
      cancelled = true
    }
  }, [initialAgentId, initialAgentTab])

  if (editingId) {
    return (
      <AgentEditView
        agentId={editingId}
        initialTab={editingId === initialAgentId ? pendingInitialTab : undefined}
        onBack={() => {
          setPendingInitialTab(undefined)
          setEditingId(null)
        }}
      />
    )
  }

  return (
    <AgentListView
      onEditAgent={(id) => setEditingId(id)}
    />
  )
}
