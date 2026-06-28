import { getTransport } from "@/lib/transport-provider"
import type { SubagentRun } from "@/types/chat"

export async function resolveBackgroundSubagentSessionId(runId: string): Promise<string | null> {
  const run = await getTransport().call<SubagentRun | null>("get_subagent_run", { runId })
  return run?.childSessionId ?? null
}
