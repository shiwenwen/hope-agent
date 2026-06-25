/**
 * Single source of truth for how an unread count is derived from a session for
 * the desktop UI. Four surfaces consume this — the per-session sidebar badge,
 * the per-type tab counts, the per-project badge, and the global indicator —
 * and they must agree on which sessions count and when. Keeping the rules here
 * prevents the surfaces from drifting (e.g. one excluding the active session
 * while another doesn't).
 *
 * Exclusions:
 *  - IM (channel) sessions → surfaced *independently* via {@link channelUnreadCount}.
 *  - Sub-agent child sessions (`parentSessionId`) → owned by their parent's UX.
 *  - The session the user is currently viewing → reads as 0 while open.
 *
 * Cron sessions never reach these counts: the backend already returns
 * `unreadCount = 0` for them (`is_cron` guard in the SQL).
 */

import type { SessionMeta } from "@/types/chat"

/**
 * Regular desktop unread for a session — what the sidebar / tab / global badges
 * sum. Returns 0 for IM, sub-agent, and the active session.
 */
export function desktopUnreadCount(
  session: SessionMeta,
  activeSessionId: string | null,
): number {
  if (session.channelInfo || session.parentSessionId) return 0
  if (session.id === activeSessionId) return 0
  return session.unreadCount
}

/**
 * IM (channel) unread for a channel-attached session. Shown as an independent
 * indicator on the session row and deliberately NOT folded into the regular
 * desktop total. Returns 0 for non-channel sessions and the active session.
 */
export function channelUnreadCount(
  session: SessionMeta,
  activeSessionId: string | null,
): number {
  if (!session.channelInfo) return 0
  if (session.id === activeSessionId) return 0
  return session.channelUnreadCount
}
