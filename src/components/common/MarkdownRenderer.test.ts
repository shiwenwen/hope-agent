import { describe, expect, test } from "vitest"
import { shouldRenderAsBareJson } from "./markdownJson"

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
