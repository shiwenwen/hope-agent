import { describe, it, expect } from "vitest"
import { aggregateSessionUrlSources } from "./useSessionUrlSources"
import type { Message } from "@/types/chat"

function webSearchMsg(result: string): Message {
  return {
    role: "assistant",
    content: "",
    contentBlocks: [
      { type: "tool_call", tool: { callId: "c", name: "web_search", arguments: "{}", result } },
    ],
  }
}

function textMsg(content: string): Message {
  return { role: "assistant", content }
}

const SEARCH_RESULT = `Search results for: cats (via google)

1. All About Cats
   URL: https://example.com/cats
   Source: example.com
   Cats are great.

2. More Cats
   URL: https://cats.org/index
   Source: cats.org
   Even more cats.`

describe("aggregateSessionUrlSources", () => {
  it("extracts URLs from a web_search tool result", () => {
    const result = aggregateSessionUrlSources([webSearchMsg(SEARCH_RESULT)])
    expect(result).toEqual([
      { kind: "url", url: "https://example.com/cats", origin: "web_search" },
      { kind: "url", url: "https://cats.org/index", origin: "web_search" },
    ])
  })

  it("extracts links from assistant message text", () => {
    const result = aggregateSessionUrlSources([textMsg("See https://docs.rs/tokio for details.")])
    expect(result).toEqual([{ kind: "url", url: "https://docs.rs/tokio", origin: "message" }])
  })

  it("dedupes a URL across sources, keeping the first origin (web_search)", () => {
    const result = aggregateSessionUrlSources([
      webSearchMsg("x\n   URL: https://example.com/cats\n"),
      textMsg("again https://example.com/cats here"),
    ])
    expect(result).toEqual([{ kind: "url", url: "https://example.com/cats", origin: "web_search" }])
  })

  it("skips private hosts and media files in message text (via extractUrls)", () => {
    const result = aggregateSessionUrlSources([
      textMsg("http://localhost:3000 and https://site.com/a.png and https://ok.com/page"),
    ])
    expect(result).toEqual([{ kind: "url", url: "https://ok.com/page", origin: "message" }])
  })

  it("extracts URLs from user message text, including asset-like URLs", () => {
    const result = aggregateSessionUrlSources([
      { role: "user", content: "please use https://example.com/report.pdf" },
    ])
    expect(result).toEqual([
      { kind: "url", url: "https://example.com/report.pdf", origin: "user_url" },
    ])
  })

  it("collects user attachments as sources", () => {
    const result = aggregateSessionUrlSources([
      {
        role: "user",
        content: "",
        attachments: [
          {
            name: "brief.pdf",
            mimeType: "application/pdf",
            sizeBytes: 1234,
            kind: "file",
            localPath: "/tmp/brief.pdf",
          },
          {
            name: "quoted.ts",
            mimeType: "text/plain",
            sizeBytes: 0,
            kind: "quote",
            quotePath: "/repo/quoted.ts",
            quoteLines: "10-12",
            quoteContent: "const x = 1",
          },
        ],
      },
    ])
    expect(result).toEqual([
      {
        kind: "attachment",
        origin: "user_attachment",
        name: "brief.pdf",
        mimeType: "application/pdf",
        sizeBytes: 1234,
        attachmentKind: "file",
        localPath: "/tmp/brief.pdf",
      },
      {
        kind: "attachment",
        origin: "user_attachment",
        name: "quoted.ts",
        mimeType: "text/plain",
        sizeBytes: 0,
        attachmentKind: "quote",
        quotePath: "/repo/quoted.ts",
        quoteLines: "10-12",
        quoteContent: "const x = 1",
      },
    ])
  })

  it("ignores non-web_search tool calls", () => {
    const msg: Message = {
      role: "assistant",
      content: "",
      contentBlocks: [
        { type: "tool_call", tool: { callId: "c", name: "read", arguments: "{}", result: "URL: https://nope.com/x" } },
      ],
    }
    expect(aggregateSessionUrlSources([msg])).toEqual([])
  })
})
