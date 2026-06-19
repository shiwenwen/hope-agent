export const PAGE_SIZE = 100
export const SESSION_PAGE_SIZE = 50

/** Per-project session page size in the sidebar. Each project group fetches
 *  its own sessions independently (`list_project_sessions_cmd`), starting at
 *  this count; "show more" loads another page, "show less" returns to one. */
export const PROJECT_SESSION_PAGE_SIZE = 15

/** Cap on the result set returned by `search_sessions_cmd` /
 *  `search_session_messages_cmd`. Beyond this the UI shows a "refine the
 *  query" hint. Shared between the sidebar global search and the in-chat
 *  find-in-page bar so behaviour stays consistent. */
export const SEARCH_LIMIT = 200

/** Per-session message-cache LRU capacity. The active session is always
 *  protected; non-active sessions evict in FIFO order beyond this limit. */
export const SESSION_CACHE_LRU_LIMIT = 5

/** Default ceiling for an in-memory session's `messages` array (bubble
 *  count, not byte size). Acts as a runaway-protection floor; the effective
 *  cap is dynamic = MAX_MESSAGES + userPaginatedDepth so anything the user
 *  actively pulled in via load-more stays headroom. */
export const MAX_MESSAGES = 1000

/** When the dynamic cap is exceeded, retain the tail of this length plus
 *  the user's paginate high-watermark. */
export const KEEP_AFTER_CAP = 800
