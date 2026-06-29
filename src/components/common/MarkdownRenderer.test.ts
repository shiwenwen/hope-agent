import { describe, expect, test } from "vitest"
import { faviconPageUrlForHref } from "@/lib/favicon"
import { shouldRenderAsBareJson } from "./markdownJson"

describe("faviconPageUrlForHref", () => {
  test("normalizes web links for safe backend favicon lookup", () => {
    expect(faviconPageUrlForHref("https://example.com/docs/page?tab=1#intro")).toBe(
      "https://example.com/",
    )
    expect(faviconPageUrlForHref("http://localhost:5173/app")).toBe("http://localhost:5173/")
  })

  test("ignores non-web and incomplete links", () => {
    expect(faviconPageUrlForHref("mailto:hello@example.com")).toBeNull()
    expect(faviconPageUrlForHref("#heading")).toBeNull()
    expect(faviconPageUrlForHref("streamdown:incomplete-link")).toBeNull()
  })
})

describe("shouldRenderAsBareJson", () => {
  test("detects complete and streaming bare JSON objects", () => {
    expect(shouldRenderAsBareJson('{"status":{"supported":true}}')).toBe(true)
    expect(shouldRenderAsBareJson('{\n  "status": {\n    "supported": true')).toBe(true)
  })

  test("detects bare JSON arrays without treating Markdown links as JSON", () => {
    expect(shouldRenderAsBareJson('[{"id":"el_1"}]')).toBe(true)
    expect(shouldRenderAsBareJson("[link](https://example.com)")).toBe(false)
  })

  test("leaves fenced code and prose on the Markdown path", () => {
    expect(shouldRenderAsBareJson('```json\n{"ok":true}\n```')).toBe(false)
    expect(shouldRenderAsBareJson('Result:\n{\n  "ok": true\n}')).toBe(false)
  })
})
