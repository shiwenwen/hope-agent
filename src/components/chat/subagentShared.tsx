import type React from "react"
import { useEffect, useState } from "react"
import { CheckCircle, XCircle, Clock, Loader2, Skull } from "lucide-react"
import { getTransport } from "@/lib/transport-provider"
import type { AgentSummaryForSidebar } from "@/types/chat"

const EMPTY_AGENTS_MAP: ReadonlyMap<string, AgentSummaryForSidebar> = new Map()

/** Shared agent metadata map (id → name/emoji) for chips, rows, breadcrumbs.
 *  Backed by the coalesced `loadAgents` cache; the initial empty map keeps a
 *  stable identity until the first load resolves. */
export function useAgentsMap(): ReadonlyMap<string, AgentSummaryForSidebar> {
  const [map, setMap] = useState<ReadonlyMap<string, AgentSummaryForSidebar>>(EMPTY_AGENTS_MAP)
  useEffect(() => {
    let alive = true
    loadAgents()
      .then((m) => {
        if (alive) setMap(m)
      })
      .catch(() => {})
    return () => {
      alive = false
    }
  }, [])
  return map
}

// ── Shared agent metadata cache (module-level, cross-instance) ─────────
// Coalesces list_agents calls across all sub-agent chips / panels via a
// single in-flight promise + 30s TTL.
let agentCache: Map<string, AgentSummaryForSidebar> | null = null
let agentCacheAt = 0
let inflight: Promise<Map<string, AgentSummaryForSidebar>> | null = null
const AGENT_CACHE_TTL_MS = 30_000

export function loadAgents(): Promise<Map<string, AgentSummaryForSidebar>> {
  const now = Date.now()
  if (agentCache && now - agentCacheAt < AGENT_CACHE_TTL_MS) {
    return Promise.resolve(agentCache)
  }
  if (inflight) return inflight
  inflight = getTransport()
    .call<AgentSummaryForSidebar[]>("list_agents")
    .then((list) => {
      agentCache = new Map(list.map((a) => [a.id, a]))
      agentCacheAt = Date.now()
      inflight = null
      return agentCache
    })
    .catch((e) => {
      inflight = null
      throw e
    })
  return inflight
}

/** `modelUsed` is stored as `<providerId>::<modelId>`. The provider half is an
 *  opaque UUID, so callers that only have this string show the model alone and
 *  resolve the provider's display name from the session meta instead. */
export function splitModelRef(modelUsed?: string | null): {
  providerId?: string
  modelId?: string
} {
  if (!modelUsed) return {}
  const sep = modelUsed.lastIndexOf("::")
  if (sep < 0) return { modelId: modelUsed.trim() || undefined }
  return {
    providerId: modelUsed.slice(0, sep).trim() || undefined,
    modelId: modelUsed.slice(sep + 2).trim() || undefined,
  }
}

export function formatModelLabel(modelUsed?: string | null): string | undefined {
  if (!modelUsed) return undefined
  return splitModelRef(modelUsed).modelId || modelUsed
}

// ── Status classification ──────────────────────────────────────────────
export const TERMINAL_STATUSES = new Set(["completed", "error", "timeout", "killed"])
export const FAILED_STATUSES = new Set(["error", "timeout", "killed"])

export interface StatusDisplay {
  icon: React.ReactNode
  color: string
}

// Status label text lives in i18n (executionStatus.subagent.status.*), not
// here — call t(`executionStatus.subagent.status.${status}`) at the use site.
export const statusConfig: Record<string, StatusDisplay> = {
  queued: {
    icon: <Clock className="h-3 w-3" />,
    color: "text-muted-foreground",
  },
  spawning: {
    icon: <Loader2 className="h-3 w-3 animate-spin" />,
    color: "text-blue-500",
  },
  running: {
    icon: <Loader2 className="h-3 w-3 animate-spin" />,
    color: "text-blue-500",
  },
  completed: {
    icon: <CheckCircle className="h-3 w-3" />,
    color: "text-green-500",
  },
  error: { icon: <XCircle className="h-3 w-3" />, color: "text-red-500" },
  timeout: { icon: <Clock className="h-3 w-3" />, color: "text-orange-500" },
  killed: { icon: <Skull className="h-3 w-3" />, color: "text-gray-500" },
}
