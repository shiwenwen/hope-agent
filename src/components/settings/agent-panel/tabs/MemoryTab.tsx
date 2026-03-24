import MemoryPanel from "@/components/settings/MemoryPanel"

interface MemoryTabProps {
  agentId: string
}

export default function MemoryTab({ agentId }: MemoryTabProps) {
  return <MemoryPanel agentId={agentId} compact />
}
