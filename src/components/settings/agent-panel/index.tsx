import { useState } from "react"
import AgentListView from "./AgentListView"
import AgentEditView from "./AgentEditView"

export default function AgentPanel({ initialAgentId }: { initialAgentId?: string }) {
  const [editingId, setEditingId] = useState<string | null>(initialAgentId ?? null)

  if (editingId) {
    return (
      <AgentEditView
        agentId={editingId}
        onBack={() => setEditingId(null)}
      />
    )
  }

  return (
    <AgentListView
      onEditAgent={(id) => setEditingId(id)}
    />
  )
}
