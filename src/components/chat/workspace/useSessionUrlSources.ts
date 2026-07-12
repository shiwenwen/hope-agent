import { useMemo } from "react"
import type { Message, MessageAttachment } from "@/types/chat"
import { extractUrls } from "@/lib/urlDetect"
import { iterateMessageToolCalls } from "./useSessionFileChanges"

/** URL 的来源：web_search 命中(结构化)、助手正文链接、或用户显式发送的链接。 */
export type UrlSourceOrigin = "web_search" | "message" | "user_url"
export type AttachmentSourceOrigin = "user_attachment"

export interface SessionUrlLinkSource {
  kind: "url"
  url: string
  origin: UrlSourceOrigin
}

export interface SessionAttachmentSource {
  kind: "attachment"
  origin: AttachmentSourceOrigin
  name: string
  mimeType: string
  sizeBytes: number
  attachmentKind: MessageAttachment["kind"]
  localPath?: string
  url?: string
  previewUrl?: string
  quotePath?: string
  quoteLines?: string
  quoteContent?: string
}

export type SessionUrlSource = SessionUrlLinkSource | SessionAttachmentSource

// web_search 工具结果是纯文本，每条命中含一行 `   URL: https://...`。
// provider 输出格式若变动这里会漏抓——属已知局限，降级为只收正文链接。
const WEB_SEARCH_URL_RE = /URL:\s*(https?:\/\/\S+)/gi
const USER_URL_RE = /https?:\/\/[^\s<>"')\]]+/gi
const TRAILING_URL_PUNCTUATION_RE = /[.,;:!?)\]]+$/
const URL_ORIGIN_PRIORITY: Record<UrlSourceOrigin, number> = {
  message: 1,
  user_url: 2,
  web_search: 3,
}

function assistantText(message: Message): string {
  if (message.contentBlocks?.length) {
    return message.contentBlocks
      .filter((b): b is { type: "text"; content: string; interrupted?: boolean } => b.type === "text")
      .map((b) => b.content)
      .join("\n")
  }
  return message.content ?? ""
}

function normalizeUrl(rawUrl: string): string {
  return rawUrl.replace(TRAILING_URL_PUNCTUATION_RE, "")
}

function extractUserSentUrls(text: string): string[] {
  const matches = text.match(USER_URL_RE)
  if (!matches) return []
  const seen = new Set<string>()
  const urls: string[] = []
  for (const raw of matches) {
    const url = normalizeUrl(raw)
    if (!url || seen.has(url)) continue
    seen.add(url)
    urls.push(url)
  }
  return urls
}

export function sessionSourceKey(source: SessionUrlSource): string {
  if (source.kind === "attachment") {
    return [
      "attachment",
      source.localPath ?? source.url ?? source.quotePath ?? source.name,
      source.quoteLines ?? "",
      source.sizeBytes,
    ].join(":")
  }
  return `url:${source.url}`
}

/**
 * 聚合本会话引用到的 URL 来源：① web_search 工具结果里命中的链接(结构来源，
 * 优先)；② 助手正文里出现的链接；③ 用户显式发送的链接；④ 用户发送的附件。
 * URL 按地址去重并保留最高优先级 origin；附件按可打开位置 / 名称去重。纯函数。
 */
export function aggregateSessionUrlSources(messages: Message[]): SessionUrlSource[] {
  const urlByValue = new Map<string, SessionUrlLinkSource>()
  const seenAttachments = new Set<string>()
  const sources: SessionUrlSource[] = []

  const add = (rawUrl: string, origin: UrlSourceOrigin) => {
    const url = normalizeUrl(rawUrl)
    if (!url) return
    const existing = urlByValue.get(url)
    if (existing) {
      if (URL_ORIGIN_PRIORITY[origin] > URL_ORIGIN_PRIORITY[existing.origin]) {
        existing.origin = origin
      }
      return
    }
    const source: SessionUrlLinkSource = { kind: "url", url, origin }
    urlByValue.set(url, source)
    sources.push(source)
  }

  const addAttachment = (attachment: MessageAttachment) => {
    const source: SessionAttachmentSource = {
      kind: "attachment",
      origin: "user_attachment",
      name: attachment.name,
      mimeType: attachment.mimeType,
      sizeBytes: attachment.sizeBytes,
      attachmentKind: attachment.kind,
      ...(attachment.localPath ? { localPath: attachment.localPath } : {}),
      ...(attachment.url ? { url: attachment.url } : {}),
      ...(attachment.previewUrl ? { previewUrl: attachment.previewUrl } : {}),
      ...(attachment.quotePath ? { quotePath: attachment.quotePath } : {}),
      ...(attachment.quoteLines ? { quoteLines: attachment.quoteLines } : {}),
      ...(attachment.quoteContent ? { quoteContent: attachment.quoteContent } : {}),
    }
    const key = sessionSourceKey(source)
    if (seenAttachments.has(key)) return
    seenAttachments.add(key)
    sources.push(source)
  }

  for (const message of messages) {
    for (const tool of iterateMessageToolCalls(message)) {
      if (tool.name !== "web_search" || !tool.result) continue
      for (const match of tool.result.matchAll(WEB_SEARCH_URL_RE)) {
        add(match[1], "web_search")
      }
    }
    if (message.role === "assistant") {
      for (const url of extractUrls(assistantText(message))) {
        add(url, "message")
      }
    } else if (message.role === "user") {
      for (const url of extractUserSentUrls(message.content ?? "")) {
        add(url, "user_url")
      }
      for (const attachment of message.attachments ?? []) {
        if (attachment.kind === "message_quote") continue
        addAttachment(attachment)
      }
    }
  }

  return sources
}

/**
 * 便宜的存在性检查：本会话有没有可能的 URL 来源。供 ChatScreen 算「是否自动展开
 * 工作台」用，短路即返回。正文用 `includes("http")` 粗判(宁可多报，避免每帧跑
 * 完整 extractUrls 正则)；调用方通常先短路在 task / file 上，很少走到这里。
 */
export function messagesHaveUrlActivity(messages: Message[]): boolean {
  for (const message of messages) {
    for (const tool of iterateMessageToolCalls(message)) {
      if (tool.name === "web_search" && tool.result) return true
    }
    if (message.role === "assistant" && assistantText(message).includes("http")) return true
    if (message.role === "user") {
      if ((message.content ?? "").includes("http")) return true
      if (message.attachments?.length) return true
    }
  }
  return false
}

export function useSessionUrlSources(messages: Message[]): SessionUrlSource[] {
  return useMemo(() => aggregateSessionUrlSources(messages), [messages])
}
