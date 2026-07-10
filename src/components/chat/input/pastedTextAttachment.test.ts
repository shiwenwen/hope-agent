import { describe, expect, test } from "vitest"

import {
  createPastedTextAttachment,
  getPastedTextFileMeta,
  PASTED_TEXT_ATTACHMENT_SOURCE,
  shouldCreatePastedTextAttachment,
  updatePastedTextAttachment,
} from "./pastedTextAttachment"

describe("pastedTextAttachment", () => {
  test("only turns long pasted text into an attachment", () => {
    expect(shouldCreatePastedTextAttachment("short text")).toBe(false)
    expect(shouldCreatePastedTextAttachment(Array.from({ length: 30 }, () => "x").join("\n"))).toBe(
      true,
    )
    expect(shouldCreatePastedTextAttachment("x".repeat(4_000))).toBe(true)
  })

  test("stores metadata on the generated File", async () => {
    const text = "# THIS IS A LONG NOTE\n" + "body\n".repeat(40)
    const file = createPastedTextAttachment(text)
    const meta = getPastedTextFileMeta(file)

    expect(file.type).toBe("text/plain")
    expect(file.name).toBe("# THIS IS A LONG NOTE.txt")
    expect(await file.text()).toBe(text)
    expect(meta?.source).toBe(PASTED_TEXT_ATTACHMENT_SOURCE)
    expect(meta?.lineCount).toBe(42)
  })

  test("keeps pasted text metadata when edited", async () => {
    const file = createPastedTextAttachment("title\n" + "body\n".repeat(35))
    const updated = updatePastedTextAttachment(file, "edited\nbody")
    const meta = getPastedTextFileMeta(updated)

    expect(updated.name).toBe(file.name)
    expect(updated.type).toBe("text/plain")
    expect(await updated.text()).toBe("edited\nbody")
    expect(meta?.source).toBe(PASTED_TEXT_ATTACHMENT_SOURCE)
    expect(meta?.lineCount).toBe(2)
    expect(meta?.charCount).toBe("edited\nbody".length)
  })
})
