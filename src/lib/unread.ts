/**
 * Single source of truth for whether a session is unread in the desktop UI.
 * Four surfaces consume this — the per-session sidebar badge,
 * the per-type tab counts, the per-project badge, and the global indicator —
 * and they must agree on which sessions count and when. Keeping the rules here
 * prevents the surfaces from drifting (e.g. one excluding the readable session
 * while another doesn't).
 *
 * Exclusions:
 *  - IM (channel) sessions → surfaced *independently* via {@link channelUnreadCount}.
 *  - Sub-agent child sessions (`parentSessionId`) → owned by their parent's UX.
 *  - The session the caller confirms is actually readable → displays as 0.
 *
 * Cron sessions never reach these counts: the backend already returns
 * `unreadCount = 0` for them (`is_cron` guard in the SQL).
 */

import type { SessionMeta } from "@/types/chat"

/**
 * Regular desktop unread flag — aggregators sum these flags to count unread
 * conversations, never unread messages. Returns 0 for IM, sub-agent, cron,
 * dedicated-space, incognito, and the actually-readable session.
 */
export function desktopUnreadCount(session: SessionMeta, readableSessionId: string | null): number {
  if (
    session.channelInfo ||
    session.parentSessionId ||
    session.isCron ||
    session.incognito ||
    (session.kind !== undefined && session.kind !== "regular")
  ) {
    return 0
  }
  if (session.id === readableSessionId) return 0
  return session.unreadCount > 0 ? 1 : 0
}

/**
 * IM (channel) unread for a channel-attached session. Shown as an independent
 * indicator on the session row and deliberately NOT folded into the regular
 * desktop total. Returns 0 for non-channel sessions and the readable session.
 */
export function channelUnreadCount(session: SessionMeta, readableSessionId: string | null): number {
  if (!session.channelInfo) return 0
  if (session.id === readableSessionId) return 0
  return session.channelUnreadCount > 0 ? 1 : 0
}
