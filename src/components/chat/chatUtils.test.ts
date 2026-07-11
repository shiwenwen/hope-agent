import { describe, expect, test, vi } from "vitest"
import type { ContentBlock, Message, SessionMessage } from "@/types/chat"
import type { Transport } from "@/lib/transport"
import { setTransport } from "@/lib/transport-provider"
import {
  computeContextUsage,
  isCenteredSystemMessage,
  isUserAlignedMessage,
  extractMessageFileAttachments,
  mergeMessagesByDbId,
  parseSessionMessages,
  reloadAndMergeSessionMessages,
  shouldSendDraftWorkflowMode,
} from "./chatUtils"

describe("shouldSendDraftWorkflowMode", () => {
  test("only forwards an enabled mode for a non-incognito new session", () => {
    expect(shouldSendDraftWorkflowMode(null, false, "on")).toBe(true)
    expect(shouldSendDraftWorkflowMode(null, false, "ultracode")).toBe(true)
    expect(shouldSendDraftWorkflowMode(null, false, "off")).toBe(false)
    expect(shouldSendDraftWorkflowMode(null, true, "on")).toBe(false)
    expect(shouldSendDraftWorkflowMode("session-1", false, "on")).toBe(false)
  })
})

function sessionMessage(patch: Partial<SessionMessage>): SessionMessage {
  return {
    id: 1,
    sessionId: "s1",
    role: "assistant",
    content: "",
    timestamp: "2026-05-12T00:00:00.000Z",
    ...patch,
  }
}

describe("parseSessionMessages events", () => {
  test("only collapses duplicate startup recovery events within one user turn", () => {
    const notice = "上次会话异常中断,已保留中断前的内容"
    const parsed = parseSessionMessages([
      sessionMessage({ id: 1, role: "event", content: notice }),
      sessionMessage({ id: 2, role: "assistant", content: "partial answer" }),
      sessionMessage({ id: 3, role: "event", content: notice }),
      sessionMessage({ id: 4, role: "event", content: "另一个事件" }),
      sessionMessage({ id: 5, role: "event", content: "另一个事件" }),
      sessionMessage({ id: 6, role: "user", content: "下一轮" }),
      sessionMessage({ id: 7, role: "event", content: notice }),
    ])

    expect(parsed.map((msg) => `${msg.role}:${msg.content}`)).toEqual([
      `event:${notice}`,
      "assistant:partial answer",
      "event:另一个事件",
      "event:另一个事件",
      "user:下一轮",
      `event:${notice}`,
    ])
  })

  test("coalesces context compaction notices while assistant blocks are pending", () => {
    const summary = JSON.stringify({
      type: "context_compacted",
      data: {
        tier_applied: 3,
        description: "summarized",
        messages_affected: 17,
      },
    })
    const syncCleanup = JSON.stringify({
      type: "context_compacted",
      data: {
        tier_applied: 2,
        description: "summarization_not_applied_sync_compaction_only",
        messages_affected: 9,
      },
    })
    const parsed = parseSessionMessages([
      sessionMessage({ id: 1, role: "user", content: "检查交付物" }),
      sessionMessage({ id: 2, role: "text_block", content: "先看目录。" }),
      sessionMessage({ id: 3, role: "event", content: summary }),
      sessionMessage({ id: 4, role: "text_block", content: "继续检查。" }),
      sessionMessage({ id: 5, role: "event", content: syncCleanup }),
      sessionMessage({ id: 6, role: "assistant", content: "最终结论。" }),
    ])

    expect(parsed.map((msg) => msg.role)).toEqual(["user", "event", "assistant"])
    expect(JSON.parse(parsed[1]!.content).data.description).toBe("summarized")
    expect(parsed[2]?.contentBlocks?.map((block) => block.type)).toEqual(["text", "text", "text"])
  })

  test("restores active memory trace from assistant attachments metadata", () => {
    const parsed = parseSessionMessages([
      sessionMessage({
        id: 1,
        role: "assistant",
        content: "收到。",
        attachmentsMeta: JSON.stringify({
          active_memory: {
            summary: "用户偏好简洁中文回答。",
            selected: {
              kind: "memory",
              id: "m1",
              sourceType: "user",
              scope: "Global",
              preview: "用户偏好简洁中文回答。",
            },
            candidates: [],
            totalCandidates: 1,
            cached: false,
          },
          used_memory_refs: [
            {
              kind: "memory",
              id: "m1",
              sourceType: "user",
              scope: "Global",
              origin: "active_memory",
              role: "selected",
              preview: "用户偏好简洁中文回答。",
            },
          ],
          retrieval_planner: {
            status: "used",
            totalRefs: 1,
            rankingVersion: "source_fusion_v2",
            intent: "profile",
            maxTraceRefs: 24,
            maxCandidatesPerOrigin: 4,
            layers: [
              {
                layer: "active_memory",
                status: "used",
                refCount: 1,
                selectedCount: 1,
                candidateCount: 0,
                cached: false,
              },
            ],
          },
        }),
      }),
    ])

    expect(parsed[0]?.activeMemory).toMatchObject({
      summary: "用户偏好简洁中文回答。",
      selected: { kind: "memory", id: "m1" },
    })
    expect(parsed[0]?.usedMemoryRefs).toEqual([
      expect.objectContaining({
        kind: "memory",
        id: "m1",
        origin: "active_memory",
        role: "selected",
      }),
    ])
    expect(parsed[0]?.retrievalPlanner).toMatchObject({
      status: "used",
      totalRefs: 1,
      rankingVersion: "source_fusion_v2",
      intent: "profile",
      maxTraceRefs: 24,
      maxCandidatesPerOrigin: 4,
      layers: [expect.objectContaining({ layer: "active_memory", status: "used" })],
    })
  })

  test("sanitizes malformed memory trace metadata before rendering", () => {
    const parsed = parseSessionMessages([
      sessionMessage({
        id: 1,
        role: "assistant",
        content: "带记忆的回答。",
        attachmentsMeta: JSON.stringify({
          used_memory_refs: [
            {
              kind: "memory",
              id: "42",
              sourceType: 123,
              scope: "Global",
              origin: { bad: true },
              role: "injected",
              preview: "用户喜欢简洁说明。",
              path: "notes/profile.md",
              line: 12,
              col: 3,
              headingPath: "Profile > Preferences",
              blockId: "pref-1",
              score: Number.POSITIVE_INFINITY,
              confidence: 0.91,
            },
            { kind: "claim" },
          ],
          retrieval_planner: {
            status: "used",
            totalRefs: "bad-total",
            layers: [
              {
                layer: "static_memory",
                status: "used",
                refCount: 2,
                latencyMs: 7,
                cached: false,
              },
              {
                layer: 99,
                status: "used",
                refCount: 1,
              },
            ],
          },
        }),
      }),
    ])

    expect(parsed[0]?.usedMemoryRefs).toHaveLength(1)
    expect(parsed[0]?.usedMemoryRefs?.[0]).toMatchObject({
      kind: "memory",
      id: "42",
      scope: "Global",
      role: "injected",
      preview: "用户喜欢简洁说明。",
      path: "notes/profile.md",
      line: 12,
      col: 3,
      headingPath: "Profile > Preferences",
      blockId: "pref-1",
      confidence: 0.91,
    })
    expect(parsed[0]?.usedMemoryRefs?.[0]?.origin).toBeUndefined()
    expect(parsed[0]?.usedMemoryRefs?.[0]?.score).toBeUndefined()

    expect(parsed[0]?.retrievalPlanner).toMatchObject({
      status: "used",
      totalRefs: 2,
      layers: [
        {
          layer: "static_memory",
          status: "used",
          refCount: 2,
          skippedReason: null,
          latencyMs: 7,
          cached: false,
        },
      ],
    })
    expect(parsed[0]?.retrievalPlanner?.layers).toHaveLength(1)
  })
})

describe("extractMessageFileAttachments", () => {
  test("preserves file-change language for path previews", () => {
    const blocks: ContentBlock[] = [
      {
        type: "tool_call",
        tool: {
          callId: "c1",
          name: "edit",
          arguments: "{}",
          metadata: {
            kind: "file_change",
            path: "/repo/generated",
            action: "edit",
            linesAdded: 2,
            linesRemoved: 1,
            before: "old",
            after: "new",
            language: "typescript",
            truncated: false,
          },
        },
      },
    ]

    expect(extractMessageFileAttachments(blocks)).toEqual([
      { kind: "path", path: "/repo/generated", language: "typescript" },
    ])
  })

  test("upgrades an existing path attachment with later metadata language", () => {
    const blocks: ContentBlock[] = [
      {
        type: "tool_call",
        tool: {
          callId: "c1",
          name: "edit",
          arguments: "{}",
          mediaUrls: ["/repo/generated"],
          metadata: {
            kind: "file_change",
            path: "/repo/generated",
            action: "edit",
            linesAdded: 2,
            linesRemoved: 1,
            before: "old",
            after: "new",
            language: "typescript",
            truncated: false,
          },
        },
      },
    ]

    expect(extractMessageFileAttachments(blocks)).toEqual([
      { kind: "path", path: "/repo/generated", language: "typescript" },
    ])
  })
})

describe("computeContextUsage", () => {
  test("uses the latest persisted compacted tokens after reload", () => {
    const usage = computeContextUsage(
      [
        {
          role: "assistant",
          content: "before compact",
          usage: { lastInputTokens: 64_000 },
        },
        {
          role: "event",
          content: JSON.stringify({
            type: "context_compacted",
            data: {
              tier_applied: 3,
              tokens_before: 64_000,
              tokens_after: 12_000,
              messages_affected: 8,
              description: "summarized",
            },
          }),
        },
      ],
      128_000,
    )

    expect(usage?.usedTokens).toBe(12_000)
    expect(usage?.pct).toBe(9)
  })

  test("prefers a newer assistant usage over an older compacted event", () => {
    const usage = computeContextUsage(
      [
        {
          role: "event",
          content: JSON.stringify({
            type: "context_compacted",
            data: {
              tier_applied: 3,
              tokens_after: 12_000,
              description: "summarized",
            },
          }),
        },
        {
          role: "assistant",
          content: "after compact",
          usage: { lastInputTokens: 18_000 },
        },
      ],
      128_000,
    )

    expect(usage?.usedTokens).toBe(18_000)
  })
})

describe("parseSessionMessages user attachments", () => {
  test("restores image attachments from user attachments metadata", () => {
    const parsed = parseSessionMessages([
      sessionMessage({
        id: 7,
        role: "user",
        content: "分析下这张图",
        attachmentsMeta: JSON.stringify([
          {
            name: "lake.png",
            mime_type: "image/png",
            size: 1024,
            path: "/Users/me/.hope-agent/attachments/s1/123_lake.png",
          },
        ]),
      }),
    ])

    expect(parsed[0]).toMatchObject({
      role: "user",
      content: "分析下这张图",
      attachments: [
        {
          name: "lake.png",
          mimeType: "image/png",
          sizeBytes: 1024,
          kind: "image",
          localPath: "/Users/me/.hope-agent/attachments/s1/123_lake.png",
        },
      ],
    })
  })

  test("does not treat object-shaped metadata as user attachments", () => {
    const parsed = parseSessionMessages([
      sessionMessage({
        id: 8,
        role: "user",
        content: "approve",
        attachmentsMeta: JSON.stringify({ plan_trigger: true }),
      }),
    ])

    expect(parsed[0]?.attachments).toBeUndefined()
    expect(parsed[0]).toMatchObject({ isPlanTrigger: true })
  })

  test("parses goal_trigger meta as a normal user-aligned goal message", () => {
    const parsed = parseSessionMessages([
      sessionMessage({
        id: 18,
        role: "user",
        content: "完成文档更新",
        attachmentsMeta: JSON.stringify({ goal_trigger: true }),
      }),
    ])

    expect(parsed[0]?.attachments).toBeUndefined()
    expect(parsed[0]).toMatchObject({ isGoalTrigger: true })
    expect(isCenteredSystemMessage(parsed[0]!)).toBe(false)
    expect(isUserAlignedMessage(parsed[0]!)).toBe(true)
  })

  test("renders display-as-user slash events as user-aligned instead of centered", () => {
    const msg = {
      role: "event",
      content: "完成文档更新",
      slashEvent: { kind: "command", displayAs: "user", mode: "goal" },
    } satisfies Message

    expect(isCenteredSystemMessage(msg)).toBe(false)
    expect(isUserAlignedMessage(msg)).toBe(true)
  })

  test("renders loop slash events as user-aligned loop messages", () => {
    const msg = {
      role: "event",
      content: "every 10m: check release blockers",
      slashEvent: { kind: "command", displayAs: "user", mode: "loop" },
    } satisfies Message

    expect(isCenteredSystemMessage(msg)).toBe(false)
    expect(isUserAlignedMessage(msg)).toBe(true)
  })

  test("parses loop_trigger meta as a centered loop chip instead of a user bubble", () => {
    const parsed = parseSessionMessages([
      sessionMessage({
        id: 84,
        role: "user",
        content: "<loop_trigger><loop_id>loop_1</loop_id></loop_trigger>",
        attachmentsMeta: JSON.stringify({ loop_trigger: { run_id: "loop_run_1" } }),
      }),
    ])

    expect(parsed[0]).toMatchObject({ isLoopTrigger: true })
    expect(parsed[0]?.isSubagentResult).toBeFalsy()
    expect(isCenteredSystemMessage(parsed[0]!)).toBe(true)
    expect(isUserAlignedMessage(parsed[0]!)).toBe(false)
  })

  test("restores channel user attachments from object-shaped metadata", () => {
    const parsed = parseSessionMessages([
      sessionMessage({
        id: 82,
        role: "user",
        content: "归档这张图",
        attachmentsMeta: JSON.stringify({
          channel_inbound: {
            channelId: "telegram",
            accountId: "acct",
            chatId: "chat-1",
            senderName: "Alice",
          },
          user_attachments: [
            {
              name: "receipt.png",
              mime_type: "image/png",
              size: 4096,
              path: "/Users/me/.hope-agent/attachments/s1/receipt.png",
            },
          ],
        }),
      }),
    ])

    expect(parsed[0]).toMatchObject({
      channelInbound: {
        channelId: "telegram",
        accountId: "acct",
        chatId: "chat-1",
        senderName: "Alice",
      },
      attachments: [
        {
          name: "receipt.png",
          mimeType: "image/png",
          sizeBytes: 4096,
          kind: "image",
          localPath: "/Users/me/.hope-agent/attachments/s1/receipt.png",
        },
      ],
    })
  })

  test("parses wakeup_trigger meta as a centered wakeup chip (not a subagent result)", () => {
    const parsed = parseSessionMessages([
      sessionMessage({
        id: 81,
        role: "user",
        content: "<wakeup>...</wakeup>",
        // Backend includes run_id in the meta (so the re-queue dedup guard
        // matches); the frontend keys only on `wakeup_trigger` presence.
        attachmentsMeta: JSON.stringify({ wakeup_trigger: { run_id: "wakeup_abc" } }),
      }),
    ])

    expect(parsed[0]?.attachments).toBeUndefined()
    expect(parsed[0]).toMatchObject({ isWakeupTrigger: true })
    // Must NOT be misclassified as a sub-agent result (the bug this fixed).
    expect(parsed[0]?.isSubagentResult).toBeFalsy()
  })

  test("parses workflow_result meta as a centered workflow chip", () => {
    const parsed = parseSessionMessages([
      sessionMessage({
        id: 83,
        role: "user",
        content: "<workflow-result><state>completed</state></workflow-result>",
        attachmentsMeta: JSON.stringify({ workflow_result: { run_id: "wfr_123" } }),
      }),
    ])

    expect(parsed[0]?.attachments).toBeUndefined()
    expect(parsed[0]).toMatchObject({ isWorkflowResult: true })
    expect(parsed[0]?.isSubagentResult).toBeFalsy()
    expect(isCenteredSystemMessage(parsed[0]!)).toBe(true)
  })

  test("restores non-image user attachments as file attachments", () => {
    const parsed = parseSessionMessages([
      sessionMessage({
        id: 9,
        role: "user",
        content: "看下这个文件",
        attachmentsMeta: JSON.stringify([
          {
            name: "brief.pdf",
            mime_type: "application/pdf",
            size: 2048,
            path: "/Users/me/.hope-agent/attachments/s1/456_brief.pdf",
          },
        ]),
      }),
    ])

    expect(parsed[0]?.attachments).toEqual([
      {
        name: "brief.pdf",
        mimeType: "application/pdf",
        sizeBytes: 2048,
        kind: "file",
        localPath: "/Users/me/.hope-agent/attachments/s1/456_brief.pdf",
      },
    ])
  })

  test("restores http-rewritten user attachments from url-only metadata", () => {
    const parsed = parseSessionMessages([
      sessionMessage({
        id: 10,
        role: "user",
        content: "web 图片",
        attachmentsMeta: JSON.stringify([
          {
            name: "web.png",
            mime_type: "image/png",
            size: 512,
            url: "/api/attachments/s1/789_web.png?token=secret",
          },
        ]),
      }),
    ])

    expect(parsed[0]?.attachments).toEqual([
      {
        name: "web.png",
        mimeType: "image/png",
        sizeBytes: 512,
        kind: "image",
        url: "/api/attachments/s1/789_web.png?token=secret",
      },
    ])
  })
})

describe("reloadAndMergeSessionMessages", () => {
  test("merges against latest cache after async DB load resolves", async () => {
    let resolveLoad: ((value: [SessionMessage[], number, boolean]) => void) | undefined
    const transport = {
      call: vi.fn(
        () =>
          new Promise<[SessionMessage[], number, boolean]>((resolve) => {
            resolveLoad = resolve
          }),
      ),
    } as unknown as Transport
    setTransport(transport)

    const sessionCacheRef = {
      current: new Map<string, Message[]>([
        [
          "s1",
          [
            {
              role: "assistant",
              content: "failed partial",
              timestamp: "2026-05-12T00:00:00.000Z",
              dbId: 1,
            },
            {
              role: "event",
              content: "failed",
              timestamp: "2026-05-12T00:00:01.000Z",
              dbId: 2,
            },
          ],
        ],
      ]),
    }
    const setMessages = vi.fn()

    const reload = reloadAndMergeSessionMessages({
      sessionId: "s1",
      pageSize: 50,
      sessionCacheRef,
      setMessages,
    })

    sessionCacheRef.current.set("s1", [
      ...sessionCacheRef.current.get("s1")!,
      {
        role: "user",
        content: "继续",
        timestamp: "2026-05-12T00:00:02.000Z",
        _clientId: "user-next",
      },
      {
        role: "assistant",
        content: "",
        timestamp: "2026-05-12T00:00:03.000Z",
        _clientId: "assistant-next",
      },
    ])

    resolveLoad?.([
      [
        sessionMessage({ id: 1, content: "failed partial" }),
        sessionMessage({
          id: 2,
          role: "event",
          content: "failed",
          timestamp: "2026-05-12T00:00:01.000Z",
        }),
      ],
      2,
      false,
    ])
    await reload

    const merged = sessionCacheRef.current.get("s1")
    expect(merged?.map((msg) => msg.role)).toEqual(["assistant", "event", "user", "assistant"])
    expect(merged?.at(-1)).toMatchObject({
      role: "assistant",
      _clientId: "assistant-next",
    })
  })

  test("does not re-append placeholders already finalized by DB rows", async () => {
    let resolveLoad: ((value: [SessionMessage[], number, boolean]) => void) | undefined
    const transport = {
      call: vi.fn(
        () =>
          new Promise<[SessionMessage[], number, boolean]>((resolve) => {
            resolveLoad = resolve
          }),
      ),
    } as unknown as Transport
    setTransport(transport)

    const sessionCacheRef = {
      current: new Map<string, Message[]>([
        [
          "s1",
          [
            {
              role: "assistant",
              content: "failed partial",
              timestamp: "2026-05-12T00:00:00.000Z",
              dbId: 1,
            },
            {
              role: "event",
              content: "failed",
              timestamp: "2026-05-12T00:00:01.000Z",
              dbId: 2,
            },
          ],
        ],
      ]),
    }
    const setMessages = vi.fn()

    const reload = reloadAndMergeSessionMessages({
      sessionId: "s1",
      pageSize: 50,
      sessionCacheRef,
      setMessages,
    })

    sessionCacheRef.current.set("s1", [
      ...sessionCacheRef.current.get("s1")!,
      {
        role: "user",
        content: "继续",
        timestamp: "2026-05-12T00:00:02.000Z",
        _clientId: "user-next",
      },
      {
        role: "assistant",
        content: "",
        timestamp: "2026-05-12T00:00:03.000Z",
        _clientId: "assistant-next",
      },
    ])

    resolveLoad?.([
      [
        sessionMessage({ id: 1, content: "failed partial" }),
        sessionMessage({
          id: 2,
          role: "event",
          content: "failed",
          timestamp: "2026-05-12T00:00:01.000Z",
        }),
        sessionMessage({
          id: 3,
          role: "user",
          content: "继续",
          timestamp: "2026-05-12T00:00:02.000Z",
        }),
        sessionMessage({
          id: 4,
          role: "assistant",
          content: "new answer",
          timestamp: "2026-05-12T00:00:03.000Z",
        }),
      ],
      4,
      false,
    ])
    await reload

    const merged = sessionCacheRef.current.get("s1")
    expect(merged?.map((msg) => msg.dbId)).toEqual([1, 2, 3, 4])
    expect(merged?.find((msg) => msg.dbId === 3)).toMatchObject({
      role: "user",
      _clientId: "user-next",
    })
    expect(merged?.find((msg) => msg.dbId === 4)).toMatchObject({
      role: "assistant",
      content: "new answer",
      _clientId: "assistant-next",
    })
  })

  test("preserves active memory trace when assistant placeholder is finalized", async () => {
    let resolveLoad: ((value: [SessionMessage[], number, boolean]) => void) | undefined
    const transport = {
      call: vi.fn(
        () =>
          new Promise<[SessionMessage[], number, boolean]>((resolve) => {
            resolveLoad = resolve
          }),
      ),
    } as unknown as Transport
    setTransport(transport)

    const activeMemory: Message["activeMemory"] = {
      summary: "用户偏好简洁中文回答。",
      selected: {
        kind: "memory",
        id: "m1",
        sourceType: "user",
        scope: "Global",
        preview: "用户偏好简洁中文回答。",
      },
      candidates: [
        {
          kind: "memory",
          id: "m1",
          sourceType: "user",
          scope: "Global",
          preview: "用户偏好简洁中文回答。",
        },
      ],
      totalCandidates: 1,
      cached: false,
    }
    const usedMemoryRefs: Message["usedMemoryRefs"] = [
      {
        kind: "memory",
        id: "m1",
        sourceType: "user",
        scope: "Global",
        origin: "active_memory",
        role: "selected",
        preview: "用户偏好简洁中文回答。",
      },
    ]
    const sessionCacheRef = {
      current: new Map<string, Message[]>([
        [
          "s1",
          [
            {
              role: "user",
              content: "怎么做?",
              timestamp: "2026-05-12T00:00:02.000Z",
              _clientId: "user-next",
            },
            {
              role: "assistant",
              content: "",
              timestamp: "2026-05-12T00:00:03.000Z",
              _clientId: "assistant-next",
              activeMemory,
              usedMemoryRefs,
            },
          ],
        ],
      ]),
    }
    const setMessages = vi.fn()

    const reload = reloadAndMergeSessionMessages({
      sessionId: "s1",
      pageSize: 50,
      sessionCacheRef,
      setMessages,
    })

    resolveLoad?.([
      [
        sessionMessage({
          id: 3,
          role: "user",
          content: "怎么做?",
          timestamp: "2026-05-12T00:00:02.000Z",
        }),
        sessionMessage({
          id: 4,
          role: "assistant",
          content: "可以。",
          timestamp: "2026-05-12T00:00:03.000Z",
        }),
      ],
      2,
      false,
    ])
    await reload

    expect(sessionCacheRef.current.get("s1")?.find((msg) => msg.dbId === 4)).toMatchObject({
      role: "assistant",
      content: "可以。",
      _clientId: "assistant-next",
      activeMemory,
      usedMemoryRefs,
    })
  })

  test("prefers finalized memory trace over placeholder trace when DB row has metadata", async () => {
    let resolveLoad: ((value: [SessionMessage[], number, boolean]) => void) | undefined
    const transport = {
      call: vi.fn(
        () =>
          new Promise<[SessionMessage[], number, boolean]>((resolve) => {
            resolveLoad = resolve
          }),
      ),
    } as unknown as Transport
    setTransport(transport)

    const placeholderActiveMemory: Message["activeMemory"] = {
      summary: "placeholder summary",
      selected: {
        kind: "memory",
        id: "m1",
        sourceType: "user",
        scope: "Global",
        preview: "placeholder preview",
      },
      candidates: [
        {
          kind: "memory",
          id: "m1",
          sourceType: "user",
          scope: "Global",
          preview: "placeholder preview",
        },
      ],
      totalCandidates: 1,
      cached: false,
    }
    const placeholderUsedRefs: Message["usedMemoryRefs"] = [
      {
        kind: "memory",
        id: "m1",
        sourceType: "user",
        scope: "Global",
        origin: "active_memory",
        role: "selected",
        preview: "placeholder preview",
        score: 0.1,
      },
    ]
    const placeholderPlanner: Message["retrievalPlanner"] = {
      status: "used",
      totalRefs: 1,
      layers: [
        {
          layer: "active_memory",
          status: "used",
          refCount: 1,
          selectedCount: 1,
          latencyMs: 1,
          cached: false,
        },
      ],
    }
    const sessionCacheRef = {
      current: new Map<string, Message[]>([
        [
          "s1",
          [
            {
              role: "user",
              content: "怎么做?",
              timestamp: "2026-05-12T00:00:02.000Z",
              _clientId: "user-next",
            },
            {
              role: "assistant",
              content: "",
              timestamp: "2026-05-12T00:00:03.000Z",
              _clientId: "assistant-next",
              activeMemory: placeholderActiveMemory,
              usedMemoryRefs: placeholderUsedRefs,
              retrievalPlanner: placeholderPlanner,
            },
          ],
        ],
      ]),
    }
    const setMessages = vi.fn()

    const reload = reloadAndMergeSessionMessages({
      sessionId: "s1",
      pageSize: 50,
      sessionCacheRef,
      setMessages,
    })

    resolveLoad?.([
      [
        sessionMessage({
          id: 3,
          role: "user",
          content: "怎么做?",
          timestamp: "2026-05-12T00:00:02.000Z",
        }),
        sessionMessage({
          id: 4,
          role: "assistant",
          content: "可以。",
          timestamp: "2026-05-12T00:00:03.000Z",
          attachmentsMeta: JSON.stringify({
            active_memory: {
              summary: "final summary",
              selected: {
                kind: "memory",
                id: "m1",
                sourceType: "user",
                scope: "Global",
                preview: "final preview",
                score: 0.88,
              },
              candidates: [
                {
                  kind: "memory",
                  id: "m1",
                  sourceType: "user",
                  scope: "Global",
                  preview: "final preview",
                  score: 0.88,
                },
              ],
              totalCandidates: 1,
              cached: true,
            },
            used_memory_refs: [
              {
                kind: "memory",
                id: "m1",
                sourceType: "user",
                scope: "Global",
                origin: "active_memory",
                role: "selected",
                preview: "final preview",
                score: 0.88,
              },
            ],
            retrieval_planner: {
              status: "used",
              totalRefs: 1,
              layers: [
                {
                  layer: "active_memory",
                  status: "used",
                  refCount: 1,
                  selectedCount: 1,
                  latencyMs: 33,
                  cached: true,
                },
              ],
            },
          }),
        }),
      ],
      2,
      false,
    ])
    await reload

    const finalized = sessionCacheRef.current.get("s1")?.find((msg) => msg.dbId === 4)
    expect(finalized).toMatchObject({
      role: "assistant",
      content: "可以。",
      _clientId: "assistant-next",
      activeMemory: expect.objectContaining({ summary: "final summary", cached: true }),
      usedMemoryRefs: [expect.objectContaining({ preview: "final preview", score: 0.88 })],
      retrievalPlanner: expect.objectContaining({
        layers: [expect.objectContaining({ latencyMs: 33, cached: true })],
      }),
    })
  })

  test("updates persisted messages when only memory diagnostics change", () => {
    const existing: Message = {
      role: "assistant",
      content: "可以。",
      timestamp: "2026-05-12T00:00:03.000Z",
      dbId: 4,
      usedMemoryRefs: [
        {
          kind: "memory",
          id: "m1",
          origin: "active_memory",
          role: "selected",
          preview: "old preview",
          score: 0.1,
        },
      ],
      retrievalPlanner: {
        status: "used",
        totalRefs: 1,
        layers: [
          {
            layer: "active_memory",
            status: "used",
            refCount: 1,
            latencyMs: 1,
            cached: false,
          },
        ],
      },
    }
    const fresh: Message = {
      ...existing,
      usedMemoryRefs: [
        {
          kind: "memory",
          id: "m1",
          origin: "active_memory",
          role: "selected",
          preview: "new preview",
          score: 0.9,
        },
      ],
      retrievalPlanner: {
        status: "used",
        totalRefs: 1,
        layers: [
          {
            layer: "active_memory",
            status: "used",
            refCount: 1,
            latencyMs: 42,
            cached: false,
          },
        ],
      },
    }

    const merged = mergeMessagesByDbId([existing], [fresh])

    expect(merged[0]).not.toBe(existing)
    expect(merged[0]).toMatchObject({
      usedMemoryRefs: [expect.objectContaining({ preview: "new preview", score: 0.9 })],
      retrievalPlanner: expect.objectContaining({
        layers: [expect.objectContaining({ latencyMs: 42 })],
      }),
    })
  })

  test("preserves dbId-less messages with identical fallback fields", async () => {
    let resolveLoad: ((value: [SessionMessage[], number, boolean]) => void) | undefined
    const transport = {
      call: vi.fn(
        () =>
          new Promise<[SessionMessage[], number, boolean]>((resolve) => {
            resolveLoad = resolve
          }),
      ),
    } as unknown as Transport
    setTransport(transport)

    const localEvent = {
      role: "event",
      content: "",
      timestamp: "2026-05-12T00:00:00.000Z",
    } satisfies Message
    const persistedMessage = {
      role: "assistant",
      content: "persisted",
      timestamp: "2026-05-12T00:00:01.000Z",
      dbId: 1,
    } satisfies Message
    const sessionCacheRef = {
      current: new Map<string, Message[]>([["s1", [localEvent, persistedMessage]]]),
    }
    const setMessages = vi.fn()

    const reload = reloadAndMergeSessionMessages({
      sessionId: "s1",
      pageSize: 50,
      sessionCacheRef,
      setMessages,
    })

    sessionCacheRef.current.set("s1", [
      localEvent,
      persistedMessage,
      {
        role: "event",
        content: "",
        timestamp: "2026-05-12T00:00:00.000Z",
      },
    ])

    resolveLoad?.([[sessionMessage({ id: 1, content: "persisted" })], 1, false])
    await reload

    const merged = sessionCacheRef.current.get("s1")
    expect(merged).toHaveLength(3)
    expect(merged?.filter((msg) => msg.role === "event" && !msg.dbId)).toHaveLength(2)
  })
})
