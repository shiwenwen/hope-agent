import { useMemo } from "react"
import type { FileChangeMetadata, Message, ToolCall } from "@/types/chat"
import { extractModifiedFiles } from "@/components/chat/chatUtils"

/** 文件在本会话里是被改写过还是仅被读取。改写细节(create/edit/delete、行数)看 `diff`。 */
export type SessionFileKind = "modified" | "read"

export interface SessionFileEntry {
  path: string
  kind: SessionFileKind
  /** 改写类且有结构化 metadata 时非 null，可直接传给 `diffPanel.openDiff`。
   *  旧消息(diff-panel 特性之前、无 metadata)兜底进来的改写文件为 null。 */
  diff: FileChangeMetadata | null
  /** 只读类的文件行数；改写类为 null。 */
  readLines: number | null
  linesAdded: number
  linesRemoved: number
  /** Shiki language id when the file-change metadata provided one. */
  language?: string | null
}

/**
 * 遍历一条消息里的 tool_call —— 优先 contentBlocks、回落 legacy toolCalls。
 * 与 useSessionUrlSources 共享，避免「contentBlocks 优先」协议重复实现。
 */
export function* iterateMessageToolCalls(message: Message): Generator<ToolCall> {
  if (message.contentBlocks?.length) {
    for (const block of message.contentBlocks) {
      if (block.type === "tool_call") yield block.tool
    }
    return
  }
  for (const tool of message.toolCalls ?? []) yield tool
}

/**
 * 聚合整个会话里被工具碰到的文件：write / edit / apply_patch 的改动 + read 的只读
 * 浏览。按 path 去重，改写优先于只读，保留最近一次改写的完整 metadata。对 diff-panel
 * 特性之前的旧消息(无结构化 metadata)用 `extractModifiedFiles` 兜底补上改写文件
 * (无 diff 数据)，与消息下挂文件保持一致。结果按最近触及排序(最新在前)。纯函数。
 */
export function aggregateSessionFileChanges(messages: Message[]): SessionFileEntry[] {
  // Map 的插入顺序即「最早触及在前」；`touch` 用 delete+set 把条目移到末尾，
  // 最后整体 reverse 得到「最近触及在前」，省掉额外的 order 数组与 O(n) splice。
  const entries = new Map<string, SessionFileEntry>()
  const touch = (path: string, entry: SessionFileEntry) => {
    entries.delete(path)
    entries.set(path, entry)
  }
  const upsertWrite = (m: FileChangeMetadata) => {
    touch(m.path, {
      path: m.path,
      kind: "modified",
      diff: m,
      readLines: null,
      linesAdded: m.linesAdded,
      linesRemoved: m.linesRemoved,
      language: m.language || null,
    })
  }

  for (const message of messages) {
    for (const tool of iterateMessageToolCalls(message)) {
      const meta = tool.metadata
      if (meta?.kind === "file_change") {
        upsertWrite(meta)
      } else if (meta?.kind === "file_changes") {
        meta.changes.forEach(upsertWrite)
      } else if (meta?.kind === "file_read") {
        const existing = entries.get(meta.path)
        // 已被改写过的文件不降级为「读」，只刷新活动顺序。
        if (existing?.kind === "modified") {
          touch(meta.path, existing)
        } else {
          touch(meta.path, {
            path: meta.path,
            kind: "read",
            diff: null,
            readLines: meta.lines,
            linesAdded: 0,
            linesRemoved: 0,
            language: null,
          })
        }
      }
      // 工具产出的文件(send_attachment / image_generate / exec)经 mediaItems 下挂，
      // 不走 file metadata —— 取本地路径作为产物文件补进「输出」(桌面有 localPath；
      // HTTP 下被剥，靠后端读时聚合补)。与后端 artifacts.rs 的 __MEDIA_ITEMS__ 扫描同步。
      for (const item of tool.mediaItems ?? []) {
        if (!item.localPath) continue
        const existing = entries.get(item.localPath)
        if (existing) {
          // 已读/已改的文件又作为媒体产出：不重复登记，但若之前只是「读」则
          // 升级为「产物」(modified)——产物比读更重要，且要刷新活动顺序。
          if (existing.kind === "read") existing.kind = "modified"
          touch(item.localPath, existing)
        } else {
          touch(item.localPath, {
            path: item.localPath,
            kind: "modified",
            diff: null,
            readLines: null,
            linesAdded: 0,
            linesRemoved: 0,
            language: null,
          })
        }
      }
    }
    // 旧消息兜底：无结构化 metadata 的改写文件(write/edit/apply_patch)，从 result/
    // arguments 解析出 path(复用消息下挂文件的提取逻辑)，无 diff 数据。
    for (const path of extractModifiedFiles(message.contentBlocks ?? [])) {
      // extractModifiedFiles 也会带出 mediaUrls(http(s) url)——那是产物媒体不是源
      // 文件,跳过;只补真实文件路径。
      if (entries.has(path) || /^https?:\/\//.test(path)) continue
      touch(path, {
        path,
        kind: "modified",
        diff: null,
        readLines: null,
        linesAdded: 0,
        linesRemoved: 0,
        language: null,
      })
    }
  }

  return [...entries.values()].reverse()
}

/**
 * 便宜的存在性检查：本会话有没有产生过文件活动。供 ChatScreen 算「是否自动展开
 * 工作台」用——短路即返回，不做整段聚合，避免在流式 render hot-path 上每帧全量扫描。
 * 判定口径必须与 {@link aggregateSessionFileChanges} 对齐：结构化 metadata **或**
 * 带本地路径的媒体产物(send_attachment / image_generate / exec)都算文件活动，
 * 否则只产媒体的会话面板永不自动展开。
 */
export function messagesHaveFileActivity(messages: Message[]): boolean {
  for (const message of messages) {
    for (const tool of iterateMessageToolCalls(message)) {
      if (tool.metadata) return true
      if (tool.mediaItems?.some((item) => item.localPath)) return true
    }
  }
  return false
}

export function useSessionFileChanges(messages: Message[]): SessionFileEntry[] {
  return useMemo(() => aggregateSessionFileChanges(messages), [messages])
}
