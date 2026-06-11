import { useCallback, useEffect, useRef, useState } from "react"

import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { SpriteConfig, SpriteSuggestion, SpriteTriggers } from "@/types/knowledge"

const DOC_SEND_CAP = 8000
const EDIT_SEND_CAP = 1200

const DEFAULT_TRIGGERS: SpriteTriggers = {
  editIdle: true,
  noteOpen: true,
  conversation: true,
  periodic: false,
  paste: true,
}

/** Cheap "what changed" diff: strip the common prefix/suffix, return the middle
 *  of the new text (net-added region) + how many chars changed. */
function diffMiddle(prev: string, next: string): { changed: number; edit: string } {
  if (prev === next) return { changed: 0, edit: "" }
  let p = 0
  const max = Math.min(prev.length, next.length)
  while (p < max && prev[p] === next[p]) p++
  let s = 0
  while (s < max - p && prev[prev.length - 1 - s] === next[next.length - 1 - s]) s++
  const edit = next.slice(p, next.length - s)
  // Effective change magnitude. Use max (not sum) of the net length delta and
  // the edited middle: for a pure insertion both are equal, so summing would
  // double-count and trip the threshold at half the configured chars.
  const changed = Math.max(Math.abs(next.length - prev.length), edit.length)
  return { changed, edit }
}

interface Opts {
  kbId: string | null
  notePath: string | null
  sessionId: string | null
  agentId: string
  /** Increments on every editor change (push signal to debounce on). */
  editorRevision: number
  /** Increments when a knowledge-chat turn completes (conversation trigger). */
  conversationRevision?: number
  /** Pulls the editor's current text (lazy — only read on fire). O(1) ref read. */
  getEditorValue: () => string
  /** Recent conversation turns (newest last) for the conversation sense. */
  getRecentMessages: () => Array<{ role: string; text: string }>
  /** Panel actually visible. */
  active: boolean
}

/**
 * Knowledge-space sprite / inspiration mode (Phase 2). Owns the enable flag, the
 * several trigger occasions (each toggleable via `SpriteConfig.triggers`), and
 * the `sprite:suggestion` listener that surfaces a transient bubble.
 *
 * Trigger occasions, all posting the same `kb_sprite_observe_cmd` (the backend
 * throttles uniformly — cooldown + hourly cap + doc-hash dedup):
 * - **noteOpen**: shortly after a note opens, react to it as-is (also seeds the
 *   diff baseline so the first edit-idle fires correctly).
 * - **editIdle**: after a pause, if enough changed since the baseline.
 * - **periodic**: every `periodicSecs` while actively writing (no idle wait).
 * - **paste**: immediately on a large single insert.
 * - **conversation**: shortly after a chat turn completes.
 *
 * The backend does all throttling + the LLM call. Tuning (thresholds / senses /
 * triggers / proactivity) lives in Settings → Knowledge Space.
 */
export function useKnowledgeSprite(opts: Opts) {
  const { kbId, notePath, sessionId, agentId, editorRevision, conversationRevision, active } = opts

  const [config, setConfig] = useState<SpriteConfig | null>(null)
  const [suggestion, setSuggestion] = useState<SpriteSuggestion | null>(null)
  // True only while the backend is actually running the side_query for THIS note
  // (drives the cat's "casting" glow). Backend emits start/stop around the call.
  const [casting, setCasting] = useState(false)
  const castingTimerRef = useRef<number | null>(null)

  // Stable readers so the trigger effects don't re-arm on every render. Synced
  // in an effect (not during render) so timers always pull the latest closures.
  const getEditorValueRef = useRef(opts.getEditorValue)
  const getRecentMessagesRef = useRef(opts.getRecentMessages)
  // `null` = diff baseline not yet established for the current note. Seeded by
  // the first trigger that runs (noteOpen dwell, or the first edit-idle tick).
  const lastObservedRef = useRef<string | null>(null)
  // Last seen doc length, for cheap large-insert (paste) detection.
  const prevLenRef = useRef(0)
  useEffect(() => {
    getEditorValueRef.current = opts.getEditorValue
    getRecentMessagesRef.current = opts.getRecentMessages
  })

  const enabled = config?.enabled ?? false
  const idleSecs = config?.idleEditSecs ?? 6
  const minChange = config?.minChangeChars ?? 40
  const periodicSecs = config?.periodicSecs ?? 120
  const pasteMinChars = config?.pasteMinChars ?? 180
  const triggers = config?.triggers ?? DEFAULT_TRIGGERS
  const armed = enabled && active && !!kbId && !!notePath

  // Load the sprite config when shown + whenever it changes elsewhere (settings).
  useEffect(() => {
    if (!active) return
    const load = () =>
      getTransport()
        .call<SpriteConfig>("sprite_config_get_cmd")
        .then(setConfig)
        .catch((e) => logger.warn("knowledge", "useKnowledgeSprite::config", "load failed", e))
    void load()
    // The EventBus `config:changed` payload's `category` is unreliable for
    // routing (the main write path always tags it `"app"`; user-config/rollback
    // paths use other values), so don't filter on it — reload on any config
    // change so an external enabled/tuning edit (settings panel) reflects here.
    return getTransport().listen("config:changed", () => void load())
  }, [active])

  // Chat-bar toggle: flip + persist `enabled` (optimistic local update). Needs
  // the config loaded first (the button is hidden/disabled until then).
  const setEnabled = useCallback(
    (on: boolean) => {
      if (!config) return
      const next = { ...config, enabled: on }
      setConfig(next)
      getTransport()
        .call<SpriteConfig>("sprite_config_set_cmd", { config: next })
        .then(setConfig)
        .catch((e) => logger.warn("knowledge", "useKnowledgeSprite::toggle", "save failed", e))
    },
    [config],
  )

  const dismiss = useCallback(() => setSuggestion(null), [])

  // One observation post, shared by every trigger. Stable per note so the timer
  // effects don't re-arm on unrelated renders.
  const fire = useCallback(
    (doc: string, edit: string | undefined) => {
      getTransport()
        .call("kb_sprite_observe_cmd", {
          params: {
            sessionId: sessionId ?? undefined,
            kbId,
            notePath,
            agentId,
            docContent: doc.slice(0, DOC_SEND_CAP),
            recentEdit: edit ? edit.slice(0, EDIT_SEND_CAP) : undefined,
            recentMessages: getRecentMessagesRef.current().slice(-6),
          },
        })
        .catch((e) => logger.warn("knowledge", "useKnowledgeSprite::observe", "post failed", e))
    },
    [sessionId, kbId, notePath, agentId],
  )

  // Switching notes invalidates the diff baseline. We intentionally do NOT read
  // the editor here: this child effect runs before the parent's editor-value
  // mirror updates (and the new note's content loads async), so reading now would
  // capture the *previous* note's text. Reset to `null` and let the first trigger
  // seed the baseline from the loaded content; reset the paste length tracker too.
  useEffect(() => {
    lastObservedRef.current = null
    prevLenRef.current = 0
  }, [notePath])

  // Listen for backend suggestions, filtered to the current note (+ session).
  useEffect(() => {
    if (!armed) return
    const unlisten = getTransport().listen("sprite:suggestion", (raw) => {
      const s = raw as SpriteSuggestion | null
      if (!s || s.notePath !== notePath) return
      if (s.sessionId && sessionId && s.sessionId !== sessionId) return
      setSuggestion(s)
    })
    return unlisten
  }, [armed, notePath, sessionId])

  // Listen for the "casting" signal (LLM call in flight) for the current note.
  // A safety timeout clears it if the backend's "done" event is ever missed.
  useEffect(() => {
    const clearTimer = () => {
      if (castingTimerRef.current != null) {
        window.clearTimeout(castingTimerRef.current)
        castingTimerRef.current = null
      }
    }
    if (!armed) {
      setCasting(false)
      clearTimer()
      return
    }
    const unlisten = getTransport().listen("sprite:casting", (raw) => {
      const c = raw as { notePath?: string; sessionId?: string; active?: boolean } | null
      if (!c || c.notePath !== notePath) return
      if (c.sessionId && sessionId && c.sessionId !== sessionId) return
      clearTimer()
      setCasting(!!c.active)
      if (c.active) castingTimerRef.current = window.setTimeout(() => setCasting(false), 30000)
    })
    return () => {
      unlisten()
      clearTimer()
    }
  }, [armed, notePath, sessionId])

  // ── Trigger: note-open dwell ──
  // A short while after opening a note, react to it as-is (no edit needed) — and
  // seed the baseline so a later edit-idle diffs against the loaded content. Skips
  // if an edit already seeded the baseline, or the note is empty.
  useEffect(() => {
    if (!armed || !triggers.noteOpen) return
    const handle = window.setTimeout(() => {
      if (lastObservedRef.current !== null) return
      const doc = getEditorValueRef.current() ?? ""
      lastObservedRef.current = doc
      if (doc.trim().length === 0) return
      fire(doc, undefined)
    }, idleSecs * 1000)
    return () => window.clearTimeout(handle)
  }, [armed, notePath, triggers.noteOpen, idleSecs, fire])

  // ── Trigger: edit-idle ──
  // Debounce on editorRevision; fire when enough changed since the baseline.
  useEffect(() => {
    if (!armed || !triggers.editIdle) return
    const handle = window.setTimeout(() => {
      const doc = getEditorValueRef.current() ?? ""
      if (lastObservedRef.current === null) {
        lastObservedRef.current = doc
        return
      }
      const { changed, edit } = diffMiddle(lastObservedRef.current, doc)
      if (changed < minChange) return
      lastObservedRef.current = doc
      fire(doc, edit)
    }, idleSecs * 1000)
    return () => window.clearTimeout(handle)
  }, [armed, editorRevision, triggers.editIdle, idleSecs, minChange, fire])

  // ── Trigger: periodic while writing ──
  // Fire every `periodicSecs` if enough changed since the baseline — doesn't wait
  // for an idle pause (companions you through a long continuous writing streak).
  useEffect(() => {
    if (!armed || !triggers.periodic) return
    const id = window.setInterval(() => {
      const doc = getEditorValueRef.current() ?? ""
      if (lastObservedRef.current === null) {
        lastObservedRef.current = doc
        return
      }
      const { changed, edit } = diffMiddle(lastObservedRef.current, doc)
      if (changed < minChange) return
      lastObservedRef.current = doc
      fire(doc, edit)
    }, periodicSecs * 1000)
    return () => window.clearInterval(id)
  }, [armed, notePath, triggers.periodic, periodicSecs, minChange, fire])

  // ── Trigger: large paste / insert ──
  // Runs on every editorRevision (cheap O(1) length read). A single change that
  // adds ≥ pasteMinChars fires immediately, no idle wait. prevLen is tracked every
  // revision (even when disabled) so toggling on can't see a stale huge delta.
  useEffect(() => {
    const doc = getEditorValueRef.current() ?? ""
    const delta = doc.length - prevLenRef.current
    prevLenRef.current = doc.length
    if (!armed || !triggers.paste) return
    if (lastObservedRef.current === null) return
    if (delta >= pasteMinChars) {
      const { edit } = diffMiddle(lastObservedRef.current, doc)
      lastObservedRef.current = doc
      fire(doc, edit)
    }
  }, [armed, editorRevision, triggers.paste, pasteMinChars, fire])

  // ── Trigger: after a chat turn completes ──
  useEffect(() => {
    if (!armed || !triggers.conversation) return
    if (!conversationRevision) return
    const handle = window.setTimeout(() => {
      const doc = getEditorValueRef.current() ?? ""
      fire(doc, undefined)
    }, 1500)
    return () => window.clearTimeout(handle)
  }, [armed, conversationRevision, triggers.conversation, fire])

  // Only surface a suggestion that belongs to the currently-open note.
  const visibleSuggestion = suggestion && suggestion.notePath === notePath ? suggestion : null

  return {
    /** Whether sprite mode is on (toggled from the chat bar). `null` config = not loaded yet. */
    enabled,
    ready: config != null,
    setEnabled,
    suggestion: visibleSuggestion,
    /** Backend is running the side_query for this note → drive the cat "casting" glow. */
    casting: casting && armed,
    dismiss,
  }
}
