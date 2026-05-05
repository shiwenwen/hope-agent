export const PAGE_SIZE = 30
export const SESSION_PAGE_SIZE = 50

/** Cap on the result set returned by `search_sessions_cmd` /
 *  `search_session_messages_cmd`. Beyond this the UI shows a "refine the
 *  query" hint. Shared between the sidebar global search and the in-chat
 *  find-in-page bar so behaviour stays consistent. */
export const SEARCH_LIMIT = 200
