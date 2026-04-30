import { useSyncExternalStore } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { SKILLS_EVENTS } from "@/types/skills"
import type { SkillSummary } from "@/components/settings/types"

const LAST_SEEN_STORAGE_KEY = "ha:skills:lastSeenDraftCount"

type Listener = () => void

let _drafts: SkillSummary[] = []
let _lastSeen = readLastSeen()
let _initialized = false
const _listeners = new Set<Listener>()
let _unlistenEvent: (() => void) | null = null

function readLastSeen(): number {
  try {
    const raw = window.localStorage.getItem(LAST_SEEN_STORAGE_KEY)
    if (!raw) return 0
    const n = Number.parseInt(raw, 10)
    return Number.isFinite(n) && n >= 0 ? n : 0
  } catch {
    return 0
  }
}

function writeLastSeen(n: number) {
  try {
    window.localStorage.setItem(LAST_SEEN_STORAGE_KEY, String(n))
  } catch {
    // localStorage may be unavailable in private mode; silently ignore
  }
}

function notify() {
  _listeners.forEach((fn) => fn())
}

function draftsEqual(a: SkillSummary[], b: SkillSummary[]): boolean {
  if (a.length !== b.length) return false
  for (let i = 0; i < a.length; i++) {
    if (a[i].name !== b[i].name) return false
  }
  return true
}

async function reloadDrafts() {
  try {
    const list = await getTransport().call<SkillSummary[]>("list_draft_skills")
    const next = Array.isArray(list) ? list : []
    let changed = false
    if (!draftsEqual(_drafts, next)) {
      _drafts = next
      changed = true
    }
    // Drafts shrank (user activated/discarded) — don't keep lastSeen above
    // current count, otherwise the badge would never reappear when the count
    // grows back to the same number.
    if (_lastSeen > _drafts.length) {
      _lastSeen = _drafts.length
      writeLastSeen(_lastSeen)
      changed = true
    }
    if (changed) notify()
  } catch (e) {
    logger.error("skills", "DraftStore::reload", "Failed to load draft skills", e)
  }
}

/**
 * Initialize the global draft skills store. Idempotent — safe to call from
 * multiple mount points; second+ calls are pure no-ops (no extra IPC) so it's
 * cheap to invoke from effects with unrelated deps.
 */
export function initDraftSkillsStore() {
  if (_initialized) return
  _initialized = true
  void reloadDrafts()
  try {
    _unlistenEvent = getTransport().listen(SKILLS_EVENTS.autoReviewComplete, () => {
      void reloadDrafts()
    })
  } catch (e) {
    logger.error("skills", "DraftStore::subscribe", "Failed to subscribe to auto_review_complete", e)
  }
}

/** Tear down the store subscription. Used in tests. */
export function disposeDraftSkillsStore() {
  _unlistenEvent?.()
  _unlistenEvent = null
  _initialized = false
}

function subscribe(listener: Listener): () => void {
  _listeners.add(listener)
  return () => _listeners.delete(listener)
}

function getDrafts(): SkillSummary[] {
  return _drafts
}

function getDraftCount(): number {
  return _drafts.length
}

function getUnseenCount(): number {
  return Math.max(0, _drafts.length - _lastSeen)
}

export function markDraftsSeen() {
  if (_lastSeen === _drafts.length) return
  _lastSeen = _drafts.length
  writeLastSeen(_lastSeen)
  notify()
}

export function refreshDraftSkillsStore() {
  void reloadDrafts()
}

export function useDraftSkillsStore(): {
  drafts: SkillSummary[]
  draftCount: number
  unseenCount: number
} {
  const drafts = useSyncExternalStore(subscribe, getDrafts)
  const draftCount = useSyncExternalStore(subscribe, getDraftCount)
  const unseenCount = useSyncExternalStore(subscribe, getUnseenCount)
  return { drafts, draftCount, unseenCount }
}
