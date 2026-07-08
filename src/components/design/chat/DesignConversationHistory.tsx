import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Search, Pencil, Trash2, Check, X } from "lucide-react"

import { Input } from "@/components/ui/input"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import type { DesignChatThread } from "@/types/design"

interface Props {
  threads: DesignChatThread[]
  activeSessionId: string | null
  onSearch: (query: string) => void
  onPick: (sessionId: string) => void
  /** 重命名线程（= 改会话标题）；父层落库后刷新列表。 */
  onRename: (sessionId: string, title: string) => void
  /** 删除线程（= 删会话，级联 design_chat_threads 行 + 消息）；父层落库后刷新。 */
  onDelete: (sessionId: string) => void
  /** True when more history pages exist beyond the loaded threads. */
  hasMore: boolean
  /** Append the next page (triggered on scroll near the bottom). */
  onLoadMore: () => void
}

/**
 * History picker for design-space conversations (project-scoped, newest-active
 * first). 双击标题行内重命名；trash 两步确认删除。搜索框对线程消息跑 FTS
 * (`design_chat_threads_list_cmd`).
 */
export function DesignConversationHistory({
  threads,
  activeSessionId,
  onSearch,
  onPick,
  onRename,
  onDelete,
  hasMore,
  onLoadMore,
}: Props) {
  const { t } = useTranslation()
  const [query, setQuery] = useState("")
  const [editingId, setEditingId] = useState<string | null>(null)
  const [editValue, setEditValue] = useState("")
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null)

  const startEdit = (thread: DesignChatThread) => {
    setConfirmDeleteId(null)
    setEditingId(thread.sessionId)
    setEditValue(thread.title?.trim() || "")
  }
  const commitEdit = () => {
    const id = editingId
    if (!id) return
    const title = editValue.trim()
    if (title) onRename(id, title)
    setEditingId(null)
  }

  return (
    <div className="absolute right-0 top-full z-30 mt-1 w-[320px] rounded-xl border border-border/60 bg-popover/95 p-2 shadow-[0_8px_30px_rgb(0,0,0,0.12)] backdrop-blur-xl">
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
          {threads.map((thread) => {
            const editing = editingId === thread.sessionId
            const confirming = confirmDeleteId === thread.sessionId
            return (
              <div
                key={thread.sessionId}
                className={cn(
                  "group/row flex items-center gap-1 rounded-lg px-1.5 py-1 transition-colors",
                  thread.sessionId === activeSessionId && "bg-secondary/40",
                  !editing && "hover:bg-secondary/60",
                )}
              >
                {editing ? (
                  <>
                    <Input
                      autoFocus
                      value={editValue}
                      onChange={(e) => setEditValue(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") commitEdit()
                        else if (e.key === "Escape") setEditingId(null)
                      }}
                      className="h-6 flex-1 text-xs"
                    />
                    <IconTip label={t("common.save", "保存")}>
                      <Button
                        size="icon"
                        variant="ghost"
                        className="h-6 w-6 shrink-0"
                        onClick={commitEdit}
                      >
                        <Check className="h-3.5 w-3.5" />
                      </Button>
                    </IconTip>
                    <IconTip label={t("common.cancel", "取消")}>
                      <Button
                        size="icon"
                        variant="ghost"
                        className="h-6 w-6 shrink-0"
                        onClick={() => setEditingId(null)}
                      >
                        <X className="h-3.5 w-3.5" />
                      </Button>
                    </IconTip>
                  </>
                ) : (
                  <>
                    <button
                      type="button"
                      onClick={() => onPick(thread.sessionId)}
                      onDoubleClick={() => startEdit(thread)}
                      className="flex min-w-0 flex-1 flex-col text-left"
                    >
                      <span className="truncate text-xs font-medium">
                        {thread.title?.trim() ||
                          thread.lastSnippet?.trim() ||
                          t("design.chat.untitled", "未命名对话")}
                      </span>
                      <span className="text-[10px] tabular-nums text-muted-foreground">
                        {t("design.chat.messageCount", "{{count}} 条", {
                          count: thread.messageCount,
                        })}
                      </span>
                    </button>
                    {confirming ? (
                      <div className="flex shrink-0 items-center gap-0.5">
                        <IconTip label={t("common.confirm", "确认")}>
                          <Button
                            size="icon"
                            variant="ghost"
                            className="h-6 w-6 text-destructive"
                            onClick={() => {
                              onDelete(thread.sessionId)
                              setConfirmDeleteId(null)
                            }}
                          >
                            <Check className="h-3.5 w-3.5" />
                          </Button>
                        </IconTip>
                        <IconTip label={t("common.cancel", "取消")}>
                          <Button
                            size="icon"
                            variant="ghost"
                            className="h-6 w-6"
                            onClick={() => setConfirmDeleteId(null)}
                          >
                            <X className="h-3.5 w-3.5" />
                          </Button>
                        </IconTip>
                      </div>
                    ) : (
                      <div className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover/row:opacity-100">
                        <IconTip label={t("common.rename", "重命名")}>
                          <Button
                            size="icon"
                            variant="ghost"
                            className="h-6 w-6"
                            onClick={() => startEdit(thread)}
                          >
                            <Pencil className="h-3 w-3" />
                          </Button>
                        </IconTip>
                        <IconTip label={t("common.delete", "删除")}>
                          <Button
                            size="icon"
                            variant="ghost"
                            className="h-6 w-6 hover:text-destructive"
                            onClick={() => setConfirmDeleteId(thread.sessionId)}
                          >
                            <Trash2 className="h-3 w-3" />
                          </Button>
                        </IconTip>
                      </div>
                    )}
                  </>
                )}
              </div>
            )
          })}
        </div>
      )}
    </div>
  )
}

export default DesignConversationHistory
