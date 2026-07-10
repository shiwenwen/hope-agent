export const MEMORY_SCOPE_FOCUS_EVENT = "hope:memory-scope-focus"

const STORAGE_KEY = "hope.memory.scope.focus"
const AGENT_TAB_SET = new Set([
  "identity",
  "personality",
  "capabilities",
  "model",
  "memory",
  "subagent",
  "approval",
  "custom",
])

export type MemoryScopeAgentTab =
  | "identity"
  | "personality"
  | "capabilities"
  | "model"
  | "memory"
  | "subagent"
  | "approval"
  | "custom"

export type MemoryScopeFocusTarget =
  | { kind: "agent"; id: string; agentTab?: MemoryScopeAgentTab }
  | { kind: "project"; id: string }

export function parseMemoryScopeFocusTarget(value: unknown): MemoryScopeFocusTarget | null {
  if (!value || typeof value !== "object") return null
  const raw = value as Record<string, unknown>
  if ((raw.kind === "agent" || raw.kind === "project") && typeof raw.id === "string") {
    const id = raw.id.trim()
    if (!id) return null
    if (raw.kind === "agent") {
      const agentTab =
        typeof raw.agentTab === "string" && AGENT_TAB_SET.has(raw.agentTab)
          ? (raw.agentTab as MemoryScopeAgentTab)
          : undefined
      return agentTab ? { kind: "agent", id, agentTab } : { kind: "agent", id }
    }
    return { kind: "project", id }
  }
  return null
}

export function requestMemoryScopeFocus(target: MemoryScopeFocusTarget): void {
  if (typeof window === "undefined") return
  try {
    window.sessionStorage.setItem(STORAGE_KEY, JSON.stringify(target))
  } catch {
    /* sessionStorage may be unavailable; the live event still works. */
  }
  window.dispatchEvent(new CustomEvent(MEMORY_SCOPE_FOCUS_EVENT, { detail: target }))
}

export function consumePendingMemoryScopeFocus(): MemoryScopeFocusTarget | null {
  if (typeof window === "undefined") return null
  try {
    const raw = window.sessionStorage.getItem(STORAGE_KEY)
    if (!raw) return null
    window.sessionStorage.removeItem(STORAGE_KEY)
    return parseMemoryScopeFocusTarget(JSON.parse(raw))
  } catch {
    return null
  }
}

export function subscribeMemoryScopeFocus(
  handler: (target: MemoryScopeFocusTarget) => void,
): () => void {
  if (typeof window === "undefined") return () => {}
  const listener = (event: Event) => {
    const target = parseMemoryScopeFocusTarget((event as CustomEvent<unknown>).detail)
    if (target) handler(target)
  }
  window.addEventListener(MEMORY_SCOPE_FOCUS_EVENT, listener)
  return () => window.removeEventListener(MEMORY_SCOPE_FOCUS_EVENT, listener)
}
