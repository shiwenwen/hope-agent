import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Search } from "lucide-react"

import { Input } from "@/components/ui/input"
import { cn } from "@/lib/utils"
import type { DesignChatThread } from "@/types/design"

interface Props {
  threads: DesignChatThread[]
  activeSessionId: string | null
  onSearch: (query: string) => void
  onPick: (sessionId: string) => void
  /** True when more history pages exist beyond the loaded threads. */
  hasMore: boolean
  /** Append the next page (triggered on scroll near the bottom). */
  onLoadMore: () => void
}

/**
 * History picker for design-space conversations (project-scoped, newest-active
 * first). Mirrors `KnowledgeConversationHistory`; the search box runs an FTS
 * filter over the threads' messages (`design_chat_threads_list_cmd`).
 */
export function DesignConversationHistory({
  threads,
  activeSessionId,
  onSearch,
  onPick,
  hasMore,
  onLoadMore,
}: Props) {
  const { t } = useTranslation()
  const [query, setQuery] = useState("")

  return (
    <div className="absolute right-0 top-full z-30 mt-1 w-[300px] rounded-xl border border-border/60 bg-popover/95 p-2 shadow-[0_8px_30px_rgb(0,0,0,0.12)] backdrop-blur-xl">
      <div className="relative mb-2">
        <Search className="pointer-events-none absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
        <Input
          autoFocus
          value={query}
          onChange={(e) => {
            setQuery(e.target.value)
            onSearch(e.target.value)
          }}
          placeholder={t("design.chat.searchHistory", "搜索历史对话…")}
          className="h-8 pl-7 text-xs"
        />
      </div>

      {threads.length === 0 ? (
        <p className="py-4 text-center text-xs text-muted-foreground">
          {t("design.chat.noHistory", "还没有对话")}
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
                thread.sessionId === activeSessionId && "bg-secondary/40",
              )}
            >
              <span className="truncate text-xs font-medium">
                {thread.title?.trim() ||
                  thread.lastSnippet?.trim() ||
                  t("design.chat.untitled", "未命名对话")}
              </span>
              <span className="flex items-center gap-1 text-[10px] text-muted-foreground">
                <span className="ml-auto shrink-0 tabular-nums">
                  {t("design.chat.messageCount", "{{count}} 条", { count: thread.messageCount })}
                </span>
              </span>
            </button>
          ))}
        </div>
      )}
    </div>
  )
}

export default DesignConversationHistory
