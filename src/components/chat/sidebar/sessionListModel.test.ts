import { describe, expect, it } from "vitest"

import type { SessionMeta, SessionSearchResult } from "@/types/chat"
import {
  filterGlobalSessionSearchResults,
  filterSessionsForSidebarTab,
  GLOBAL_SESSION_SEARCH_TYPES,
  sidebarSessionPageArgs,
} from "./sessionListModel"

function session(id: string, patch: Partial<SessionMeta> = {}): SessionMeta {
  return {
    id,
    title: id,
    agentId: "agent-a",
    createdAt: "2026-07-12T00:00:00Z",
    updatedAt: "2026-07-12T00:00:00Z",
    messageCount: 1,
    unreadCount: 0,
    channelUnreadCount: 0,
    hasError: false,
    pendingInteractionCount: 0,
    isCron: false,
    incognito: false,
    ...patch,
  }
}

function searchResult(
  sessionId: string,
  patch: Partial<SessionSearchResult> = {},
): SessionSearchResult {
  return {
    messageId: 1,
    sessionId,
    sessionTitle: sessionId,
    agentId: "agent-a",
    messageRole: "assistant",
    contentSnippet: sessionId,
    timestamp: "2026-07-12T00:00:00Z",
    relevanceRank: 0,
    isCron: false,
    parentSessionId: null,
    projectId: null,
    channelType: null,
    channelChatType: null,
    matchKind: "message",
    ...patch,
  }
}

describe("sidebar session list model", () => {
  const sessions = [
    session("regular"),
    session("channel", {
      channelInfo: {
        channelId: "telegram",
        accountId: "account",
        chatId: "chat",
        chatType: "direct",
      },
    }),
    session("project", { projectId: "project-a" }),
    session("subagent", { parentSessionId: "parent" }),
    session("project-subagent", { projectId: "project-a", parentSessionId: "parent" }),
    session("cron", { isCron: true }),
    session("other-agent", { agentId: "agent-b" }),
  ]

  it("keeps the conversation tab limited to unassigned user and channel chats", () => {
    expect(filterSessionsForSidebarTab(sessions, "session").map((item) => item.id)).toEqual([
      "regular",
      "channel",
      "other-agent",
    ])
  })

  it("keeps the subagent tab limited to unassigned subagent chats", () => {
    expect(filterSessionsForSidebarTab(sessions, "subagent").map((item) => item.id)).toEqual([
      "subagent",
    ])
  })

  it("applies the agent filter to the same rows used by rendering and unread counts", () => {
    expect(
      filterSessionsForSidebarTab(sessions, "session", "agent-b").map((item) => item.id),
    ).toEqual(["other-agent"])
  })

  it("pushes project, parent, and agent filters into the paginated query", () => {
    expect(sidebarSessionPageArgs("session", "agent-b", 50, 50, "active")).toEqual({
      unassigned: true,
      parentSession: false,
      agentId: "agent-b",
      limit: 50,
      offset: 50,
      activeSessionId: "active",
    })
    expect(sidebarSessionPageArgs("subagent", null, 0, 50)).toEqual({
      unassigned: true,
      parentSession: true,
      agentId: undefined,
      limit: 50,
      offset: 0,
      activeSessionId: undefined,
    })
  })

  it("keeps global search broad across project, channel, and subagent results", () => {
    const results = [
      searchResult("regular"),
      searchResult("project", { projectId: "project-a" }),
      searchResult("channel", { channelType: "telegram" }),
      searchResult("subagent", { parentSessionId: "parent" }),
      searchResult("cron", { isCron: true }),
    ]

    expect(filterGlobalSessionSearchResults(results).map((item) => item.sessionId)).toEqual([
      "regular",
      "project",
      "channel",
      "subagent",
    ])
    expect(GLOBAL_SESSION_SEARCH_TYPES).toEqual(["regular", "subagent", "channel"])
  })
})
