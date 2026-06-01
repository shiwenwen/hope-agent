import { useMemo } from "react"
import type { Message } from "@/types/chat"
import { extractUrls } from "@/lib/urlDetect"
import { iterateMessageToolCalls } from "./useSessionFileChanges"

/** URL 的来源：web_search 命中(结构化)或助手正文里的链接。 */
export type UrlSourceOrigin = "web_search" | "message"

export interface SessionUrlSource {
  url: string
  origin: UrlSourceOrigin
}

// web_search 工具结果是纯文本，每条命中含一行 `   URL: https://...`。
// provider 输出格式若变动这里会漏抓——属已知局限，降级为只收正文链接。
const WEB_SEARCH_URL_RE = /URL:\s*(https?:\/\/\S+)/gi

function assistantText(message: Message): string {
  if (message.contentBlocks?.length) {
    return message.contentBlocks
      .filter((b): b is { type: "text"; content: string; interrupted?: boolean } => b.type === "text")
      .map((b) => b.content)
      .join("\n")
  }
  return message.content ?? ""
}

/**
 * 聚合本会话引用到的 URL 来源：① web_search 工具结果里命中的链接(结构来源，
 * 优先)；② 助手正文里出现的链接。按 url 去重，保留首次遇到的 origin。纯函数。
 */
export function aggregateSessionUrlSources(messages: Message[]): SessionUrlSource[] {
  const seen = new Set<string>()
  const sources: SessionUrlSource[] = []

  const add = (rawUrl: string, origin: UrlSourceOrigin) => {
    const url = rawUrl.replace(/[.,;:!?)\]]+$/, "")
    if (seen.has(url)) return
    seen.add(url)
    sources.push({ url, origin })
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
  }
  return false
}

export function useSessionUrlSources(messages: Message[]): SessionUrlSource[] {
  return useMemo(() => aggregateSessionUrlSources(messages), [messages])
}
