import { useState } from "react"
import TemplateListView from "./TemplateListView"
import TemplateEditView from "./TemplateEditView"

export default function TeamsPanel() {
  const [editingId, setEditingId] = useState<string | "__new__" | null>(null)

  if (editingId !== null) {
    return (
      <TemplateEditView
        templateId={editingId}
        onBack={() => setEditingId(null)}
      />
    )
  }

  return <TemplateListView onEdit={(id) => setEditingId(id)} />
}
