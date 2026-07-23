import { useTranslation } from "react-i18next"
import { Search } from "lucide-react"

import { SearchInput } from "@/components/ui/search-input"
import { FloatingMenu } from "@/components/ui/floating-menu"
import { cn } from "@/lib/utils"
import type { KbChatThread } from "@/types/knowledge"
import {
  knowledgeChatIssueDescription,
  knowledgeChatIssueTitle,
  type KnowledgeChatLoadIssue,
} from "./knowledgeChatFeedback"

interface Props {
  open: boolean
  threads: KbChatThread[]
  activeSessionId: string | null
  query: string
  onSearch: (query: string) => void
  onPick: (sessionId: string) => void
  /** True when more history pages exist beyond the loaded threads. */
  hasMore: boolean
  /** Append the next page (triggered on scroll near the bottom). */
  onLoadMore: () => void
  /** History list/page read failure; shown instead of silently empty history. */
  loadIssue?: KnowledgeChatLoadIssue | null
}

/**
 * History picker for knowledge-space conversations (KB-scoped, newest-active
 * first). Each row shows the anchor note + a preview; the search box runs an FTS
 * filter over the threads' messages (`kb_chat_threads_list_cmd`).
 */
export function KnowledgeConversationHistory({
  open,
  threads,
  activeSessionId,
  query,
  onSearch,
  onPick,
  hasMore,
  onLoadMore,
  loadIssue,
}: Props) {
  const { t } = useTranslation()
  const issueDescription = loadIssue
    ? knowledgeChatIssueDescription(loadIssue, t)
    : null

  return (
    <FloatingMenu
      open={open}
      positionClassName="top-full right-0 mt-1.5"
      originClassName="origin-top-right"
      className="ha-menu-from-top w-[300px] p-2"
    >
      <div className="relative mb-2">
        <Search className="pointer-events-none absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
        <SearchInput
          autoFocus
          value={query}
          onChange={(e) => {
            onSearch(e.target.value)
          }}
          placeholder={t("knowledge.chatPanel.searchHistory")}
          className="h-8 pl-7 text-xs"
        />
      </div>

      {loadIssue ? (
        <div className="mb-2 rounded-md border border-destructive/30 bg-destructive/5 px-2 py-1.5 text-xs text-destructive">
          <div>{knowledgeChatIssueTitle(loadIssue, t)}</div>
          {issueDescription ? (
            <div className="mt-1 break-words text-[11px] leading-relaxed text-muted-foreground">
              {issueDescription}
            </div>
          ) : null}
        </div>
      ) : null}

      {threads.length === 0 && !loadIssue ? (
        <p className="py-4 text-center text-xs text-muted-foreground">
          {t("knowledge.chatPanel.noHistory")}
        </p>
      ) : (
        <div
          className="flex max-h-[320px] flex-col gap-0.5 overflow-y-auto"
          onScroll={(e) => {
            const el = e.currentTarget
            if (hasMore && el.scrollHeight - el.scrollTop - el.clientHeight < 48) {
              onLoadMore()
            }
          }}
        >
          {threads.map((thread) => (
            <button
              key={thread.sessionId}
              onClick={() => onPick(thread.sessionId)}
              className={cn(
                "flex flex-col gap-0.5 rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-secondary/60",
                thread.sessionId === activeSessionId && "bg-secondary",
              )}
            >
              <span className="truncate text-xs font-medium">
                {thread.title?.trim() ||
                  thread.lastSnippet?.trim() ||
                  t("knowledge.chatPanel.untitled")}
              </span>
              <span className="flex items-center gap-1 text-[10px] text-muted-foreground">
                {thread.anchorNotePath && (
                  <span className="truncate">{thread.anchorNotePath}</span>
                )}
                <span className="ml-auto shrink-0 tabular-nums">
                  {t("knowledge.chatPanel.messageCount", { count: thread.messageCount })}
                </span>
              </span>
            </button>
          ))}
        </div>
      )}
    </FloatingMenu>
  )
}

export default KnowledgeConversationHistory
