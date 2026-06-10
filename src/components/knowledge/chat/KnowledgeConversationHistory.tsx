import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Search } from "lucide-react"

import { Input } from "@/components/ui/input"
import { cn } from "@/lib/utils"
import type { KbChatThread } from "@/types/knowledge"

interface Props {
  threads: KbChatThread[]
  activeSessionId: string | null
  onSearch: (query: string) => void
  onPick: (sessionId: string) => void
}

/**
 * History picker for knowledge-space conversations (KB-scoped, newest-active
 * first). Each row shows the anchor note + a preview; the search box runs an FTS
 * filter over the threads' messages (`kb_chat_threads_list_cmd`).
 */
export function KnowledgeConversationHistory({
  threads,
  activeSessionId,
  onSearch,
  onPick,
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
          placeholder={t("knowledge.chatPanel.searchHistory")}
          className="h-8 pl-7 text-xs"
        />
      </div>

      {threads.length === 0 ? (
        <p className="py-4 text-center text-xs text-muted-foreground">
          {t("knowledge.chatPanel.noHistory")}
        </p>
      ) : (
        <div className="flex max-h-[320px] flex-col gap-0.5 overflow-y-auto">
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
    </div>
  )
}

export default KnowledgeConversationHistory
