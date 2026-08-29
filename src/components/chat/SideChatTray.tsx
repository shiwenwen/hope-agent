import { MessageSquareText, Plus, X } from "lucide-react"
import { useTranslation } from "react-i18next"

import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import type { SessionMeta } from "@/types/chat"

interface SideChatTrayProps {
  chats: SessionMeta[]
  activeId: string | null
  panelOpen: boolean
  creating: boolean
  onCreate: () => void
  onSelect: (sessionId: string) => void
  onClosePanel: () => void
}

export function SideChatTray({
  chats,
  activeId,
  panelOpen,
  creating,
  onCreate,
  onSelect,
  onClosePanel,
}: SideChatTrayProps) {
  const { t } = useTranslation()

  return (
    <div className="flex min-w-0 items-center gap-1.5 border-b border-border/70 bg-muted/35 px-2 py-1.5">
      <MessageSquareText
        className="h-3.5 w-3.5 shrink-0 text-muted-foreground"
        aria-hidden="true"
      />
      <span className="shrink-0 text-xs font-medium text-muted-foreground">
        {t("chat.sideChat.label", "侧聊")}
      </span>
      <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto">
        {chats.map((chat, index) => {
          const selected = panelOpen && activeId === chat.id
          return (
            <button
              key={chat.id}
              type="button"
              aria-pressed={selected}
              onClick={() => onSelect(chat.id)}
              className={cn(
                "max-w-40 shrink-0 truncate rounded-md px-2 py-1 text-xs transition-colors",
                selected
                  ? "bg-background font-medium text-foreground"
                  : "text-muted-foreground hover:bg-background/70 hover:text-foreground",
              )}
            >
              {chat.title?.trim() ||
                t("chat.sideChat.untitled", {
                  index: index + 1,
                  defaultValue: "侧聊 {{index}}",
                })}
            </button>
          )
        })}
      </div>
      <IconTip label={t("chat.sideChat.new", "新建侧聊")}>
        <Button
          type="button"
          size="icon"
          variant="ghost"
          className="h-6 w-6 shrink-0"
          disabled={creating}
          onClick={onCreate}
          aria-label={t("chat.sideChat.new", "新建侧聊")}
        >
          <Plus className="h-3.5 w-3.5" />
        </Button>
      </IconTip>
      {panelOpen ? (
        <IconTip label={t("chat.sideChat.close", "关闭侧聊面板")}>
          <Button
            type="button"
            size="icon"
            variant="ghost"
            className="h-6 w-6 shrink-0"
            onClick={onClosePanel}
            aria-label={t("chat.sideChat.close", "关闭侧聊面板")}
          >
            <X className="h-3.5 w-3.5" />
          </Button>
        </IconTip>
      ) : null}
    </div>
  )
}
