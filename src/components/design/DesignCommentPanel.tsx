/**
 * 批注钉面板（P0 ①，右侧与 DesignInspector 并列互斥）。
 *
 * 钉本身由 iframe bridge 渲染在预览上（坐标随锚元素、zoom 无关）；本面板做数据交互：
 * 列表 / 新建（点选元素落钉后填正文）/ 标记已解决 / 编辑 / 删除 / **回灌对话让 AI 精修**。
 * 点条目 → 通知 bridge 聚焦对应钉。纯受控，父层负责 owner 命令与 iframe 通信。
 */

import { useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  MessageSquare,
  MessagesSquare,
  X,
  Check,
  Trash2,
  Pencil,
  Send,
  CornerDownLeft,
  CheckSquare,
} from "lucide-react"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { IconTip } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import type { CommentPlacement, DesignComment } from "@/types/design"

interface Props {
  comments: DesignComment[]
  /** 待填正文的新钉锚点（bridge 落钉后置入）；null = 无待填。 */
  pending: CommentPlacement | null
  onCreate: (body: string) => void
  onCancelPending: () => void
  onResolve: (id: number, resolved: boolean) => void
  onEdit: (id: number, body: string) => void
  onDelete: (id: number) => void
  onFocus: (id: number) => void
  /** 一键快捷：让 AI 按这条批注就地精修出新版本（不进对话）。 */
  onSendToChat: (id: number) => void
  /** 带到对话：把这条批注作为 quote 塞进左侧 AI 对话 composer，用户可补充后随 turn 发。 */
  onAddToChat: (id: number) => void
  /** 批量带到对话（B4-2）：把多条批注合成一个 scope-guarded 结构块塞进 composer。 */
  onBatchToChat: (ids: number[]) => void
  /** 预览里点钉请求聚焦的批注 id（B0-3）：滚动到该卡并进入编辑；消费后经 onFocusHandled 清空。 */
  focusCommentId?: number | null
  onFocusHandled?: () => void
  onClose: () => void
}

export default function DesignCommentPanel({
  comments,
  pending,
  onCreate,
  onCancelPending,
  onResolve,
  onEdit,
  onDelete,
  onFocus,
  onSendToChat,
  onAddToChat,
  onBatchToChat,
  focusCommentId,
  onFocusHandled,
  onClose,
}: Props) {
  const { t } = useTranslation()
  const [draft, setDraft] = useState("")
  const [editingId, setEditingId] = useState<number | null>(null)
  const [editDraft, setEditDraft] = useState("")
  const cardRefs = useRef<Map<number, HTMLDivElement>>(new Map())
  // 多选批量带到对话（B4-2）。
  const [selectMode, setSelectMode] = useState(false)
  const [selected, setSelected] = useState<Set<number>>(() => new Set())
  const toggleSelected = (id: number) =>
    setSelected((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  const exitSelect = () => {
    setSelectMode(false)
    setSelected(new Set())
  }

  // 新钉锚点变了（落到另一元素）→ 清空新建草稿，避免上一次输入带到新钉（review #6）。
  // 用 React「渲染期调整 state」模式而非 effect 内 setState（后者会触发级联渲染，eslint 拦）。
  const pendingKey = pending ? `${pending.oid}:${pending.relX}:${pending.relY}` : null
  const [lastPendingKey, setLastPendingKey] = useState<string | null>(pendingKey)
  if (pendingKey !== lastPendingKey) {
    setLastPendingKey(pendingKey)
    setDraft("")
  }

  // 预览点钉 → 聚焦该批注：滚动到卡片、进入编辑，消费后通知父层清空（B0-3）。
  // state 变更延到下一帧，避免 effect 内同步 setState 触发级联渲染（仓库 eslint 拦）。
  useEffect(() => {
    if (focusCommentId == null) return
    cardRefs.current
      .get(focusCommentId)
      ?.scrollIntoView({ behavior: "smooth", block: "center" })
    const c = comments.find((x) => x.id === focusCommentId)
    const raf = requestAnimationFrame(() => {
      if (c && !c.resolved) {
        setEditingId(c.id)
        setEditDraft(c.body)
      }
      onFocusHandled?.()
    })
    return () => cancelAnimationFrame(raf)
  }, [focusCommentId, comments, onFocusHandled])

  const open = comments.filter((c) => !c.resolved)
  const resolved = comments.filter((c) => c.resolved)

  const submitNew = () => {
    const body = draft.trim()
    if (!body) return
    onCreate(body)
    setDraft("")
  }

  const startEdit = (c: DesignComment) => {
    setEditingId(c.id)
    setEditDraft(c.body)
  }
  const submitEdit = (id: number) => {
    const body = editDraft.trim()
    if (body) onEdit(id, body)
    setEditingId(null)
  }

  const renderCard = (c: DesignComment, index: number) => (
    <div
      key={c.id}
      ref={(el) => {
        if (el) cardRefs.current.set(c.id, el)
        else cardRefs.current.delete(c.id)
      }}
      className={cn(
        "group rounded-lg border p-2.5 text-sm transition-colors",
        c.resolved ? "bg-muted/40 opacity-70" : "bg-card hover:bg-secondary/40",
      )}
    >
      <div className="flex items-start gap-2">
        {selectMode && !c.resolved && (
          <button
            type="button"
            onClick={() => toggleSelected(c.id)}
            aria-label={t("design.selectMultiple", "多选")}
            className={cn(
              "mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded border-2 transition-colors",
              selected.has(c.id)
                ? "border-transparent bg-primary text-primary-foreground"
                : "border-border",
            )}
          >
            {selected.has(c.id) && <Check className="h-3 w-3" />}
          </button>
        )}
        <IconTip label={t("design.comment.locate", "定位到钉")}>
          <button
            type="button"
            onClick={() => (selectMode && !c.resolved ? toggleSelected(c.id) : onFocus(c.id))}
            className={cn(
              "mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-[11px] font-semibold text-white",
              c.resolved ? "bg-emerald-600" : "bg-amber-500",
            )}
          >
            {index + 1}
          </button>
        </IconTip>
        <div className="min-w-0 flex-1">
          {(c.snippet || c.tag) && (
            <div className="mb-1 flex items-center gap-1.5 text-[10px] text-muted-foreground">
              {c.oid == null && (
                <span className="shrink-0 text-amber-500">
                  ⚠ {t("design.comment.detached", "已脱锚")}
                </span>
              )}
              {c.tag && (
                <span className="shrink-0 rounded bg-muted px-1 py-px font-mono text-[9px] uppercase">
                  {c.tag}
                </span>
              )}
              {c.snippet && <span className="truncate">{c.snippet}</span>}
            </div>
          )}
          {editingId === c.id ? (
            <div className="space-y-1.5">
              <Textarea
                value={editDraft}
                onChange={(e) => setEditDraft(e.target.value)}
                onKeyDown={(e) => {
                  // 编辑态同款键盘契约（W3-J：此前编辑框零键盘处理、只能点按钮）。
                  if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                    e.preventDefault()
                    submitEdit(c.id)
                  } else if (e.key === "Escape") {
                    e.preventDefault()
                    setEditingId(null)
                  }
                }}
                rows={2}
                className="text-sm"
                autoFocus
              />
              <div className="flex gap-1.5">
                <Button size="sm" className="h-6 px-2 text-xs" onClick={() => submitEdit(c.id)}>
                  {t("common.save", "保存")}
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  className="h-6 px-2 text-xs"
                  onClick={() => setEditingId(null)}
                >
                  {t("common.cancel", "取消")}
                </Button>
              </div>
            </div>
          ) : (
            <div className={cn("whitespace-pre-wrap break-words", c.resolved && "line-through")}>
              {c.body}
            </div>
          )}
        </div>
      </div>
      {editingId !== c.id && !selectMode && (
        <div className="mt-1.5 flex items-center justify-end gap-0.5 opacity-60 transition-opacity group-hover:opacity-100">
          <IconTip label={t("design.comment.addToChat", "带到对话")}>
            <Button
              size="icon"
              variant="ghost"
              className="h-6 w-6"
              onClick={() => onAddToChat(c.id)}
            >
              <MessagesSquare className="h-3.5 w-3.5" />
            </Button>
          </IconTip>
          <IconTip label={t("design.comment.sendToChat", "一键精修")}>
            <Button
              size="icon"
              variant="ghost"
              className="h-6 w-6"
              onClick={() => onSendToChat(c.id)}
            >
              <Send className="h-3.5 w-3.5" />
            </Button>
          </IconTip>
          <IconTip
            label={
              c.resolved
                ? t("design.comment.reopen", "取消解决")
                : t("design.comment.resolve", "标记已解决")
            }
          >
            <Button
              size="icon"
              variant="ghost"
              className={cn("h-6 w-6", c.resolved && "text-emerald-600")}
              onClick={() => onResolve(c.id, !c.resolved)}
            >
              <Check className="h-3.5 w-3.5" />
            </Button>
          </IconTip>
          <IconTip label={t("common.edit", "编辑")}>
            <Button size="icon" variant="ghost" className="h-6 w-6" onClick={() => startEdit(c)}>
              <Pencil className="h-3.5 w-3.5" />
            </Button>
          </IconTip>
          <IconTip label={t("common.delete", "删除")}>
            <Button
              size="icon"
              variant="ghost"
              className="h-6 w-6 text-destructive"
              onClick={() => onDelete(c.id)}
            >
              <Trash2 className="h-3.5 w-3.5" />
            </Button>
          </IconTip>
        </div>
      )}
    </div>
  )

  return (
    <aside className="flex w-72 shrink-0 flex-col overflow-hidden border-l bg-background">
      <div className="flex items-center justify-between border-b px-3 py-2.5">
        <div className="flex items-center gap-2 text-sm font-semibold">
          <MessageSquare className="h-4 w-4 text-amber-500" />
          {t("design.comment.title", "批注")}
          {comments.length > 0 && (
            <span className="rounded-full bg-muted px-1.5 text-[11px] font-medium text-muted-foreground">
              {comments.length}
            </span>
          )}
        </div>
        <div className="flex items-center gap-0.5">
          {open.length > 1 && (
            <IconTip label={t("design.comment.batchToChat", "多选带到对话")}>
              <Button
                size="icon"
                variant={selectMode ? "default" : "ghost"}
                className="h-6 w-6"
                onClick={() => (selectMode ? exitSelect() : setSelectMode(true))}
              >
                <CheckSquare className="h-3.5 w-3.5" />
              </Button>
            </IconTip>
          )}
          <IconTip label={t("common.close", "关闭")}>
            <Button size="icon" variant="ghost" className="h-6 w-6" onClick={onClose}>
              <X className="h-4 w-4" />
            </Button>
          </IconTip>
        </div>
      </div>

      {selectMode && (
        <div className="flex items-center gap-2 border-b bg-secondary/40 px-3 py-2 text-xs">
          <span className="text-muted-foreground">
            {t("design.selectedCount", "已选 {{count}} 项", { count: selected.size })}
          </span>
          <div className="ml-auto flex items-center gap-1.5">
            <Button size="sm" variant="ghost" className="h-6 px-2" onClick={exitSelect}>
              {t("common.cancel", "取消")}
            </Button>
            <Button
              size="sm"
              className="h-6 gap-1 px-2"
              disabled={selected.size === 0}
              onClick={() => {
                onBatchToChat([...selected])
                exitSelect()
              }}
            >
              <MessagesSquare className="h-3 w-3" />
              {t("design.comment.batchToChat", "带到对话")}
            </Button>
          </div>
        </div>
      )}

      <div className="flex-1 space-y-2 overflow-y-auto p-2.5">
        {/* 待填的新钉 */}
        {pending ? (
          <div className="rounded-lg border border-amber-400/60 bg-amber-50/50 p-2.5 dark:bg-amber-950/20">
            {(pending.snippet || pending.tag) && (
              <div className="mb-1.5 flex items-center gap-1.5 text-[10px] text-muted-foreground">
                {pending.tag && (
                  <span className="shrink-0 rounded bg-muted px-1 py-px font-mono text-[9px] uppercase">
                    {pending.tag}
                  </span>
                )}
                {pending.snippet && <span className="truncate">{pending.snippet}</span>}
              </div>
            )}
            <Textarea
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => {
                // 纯 Enter 提交（IME 组字期不误触，对齐 ⏎ 图标 + DesignDrawOverlay 契约）；Shift+Enter
                // 换行；Escape 取消待填钉（W3-J：此前只认 Cmd/Ctrl+Enter、⏎ 图标误导、无 Escape）。
                if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                  e.preventDefault()
                  submitNew()
                } else if (e.key === "Escape") {
                  e.preventDefault()
                  setDraft("")
                  onCancelPending()
                }
              }}
              rows={2}
              placeholder={t("design.comment.placeholder", "写下对这个元素的反馈…")}
              className="text-sm"
              autoFocus
            />
            <div className="mt-1.5 flex items-center gap-1.5">
              <Button size="sm" className="h-6 px-2 text-xs" onClick={submitNew} disabled={!draft.trim()}>
                {t("design.comment.add", "添加批注")}
                <CornerDownLeft className="ml-1 h-3 w-3 opacity-60" />
              </Button>
              <Button
                size="sm"
                variant="ghost"
                className="h-6 px-2 text-xs"
                onClick={() => {
                  setDraft("")
                  onCancelPending()
                }}
              >
                {t("common.cancel", "取消")}
              </Button>
            </div>
          </div>
        ) : (
          comments.length === 0 && (
            <div className="px-1 py-6 text-center text-xs text-muted-foreground">
              {t("design.comment.emptyHint", "点选预览中的元素即可留下批注")}
            </div>
          )
        )}

        {open.map((c) => renderCard(c, comments.indexOf(c)))}
        {resolved.length > 0 && (
          <div className="pt-1 text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
            {t("design.comment.resolvedSection", "已解决")} · {resolved.length}
          </div>
        )}
        {resolved.map((c) => renderCard(c, comments.indexOf(c)))}
      </div>
    </aside>
  )
}
