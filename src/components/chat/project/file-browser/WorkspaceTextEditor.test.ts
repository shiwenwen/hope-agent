import { describe, expect, test } from "vitest"

import type { FileTextContent } from "@/lib/transport"
import { dominantLineEnding, editorText, serializeText } from "./WorkspaceTextEditor"

function content(
  value: string,
  lineEnding: FileTextContent["lineEnding"],
  hasUtf8Bom = false,
): FileTextContent {
  return {
    relPath: "notes.md",
    content: value,
    isBinary: false,
    mime: "text/markdown",
    totalLines: 2,
    sizeBytes: value.length,
    truncated: false,
    contentHash: "hash",
    isUtf8: true,
    lineEnding,
    hasUtf8Bom,
  }
}

describe("WorkspaceTextEditor text format", () => {
  test("normalizes disk line endings for CodeMirror", () => {
    expect(editorText("one\r\ntwo\rthree\n")).toBe("one\ntwo\nthree\n")
  })

  test("preserves UTF-8 BOM and CRLF", () => {
    expect(serializeText("one\ntwo\n", content("one\r\ntwo\r\n", "crlf", true))).toBe(
      "\uFEFFone\r\ntwo\r\n",
    )
  })

  test("normalizes mixed endings to the dominant original style", () => {
    const original = "one\r\ntwo\r\nthree\nfour\r\n"
    expect(dominantLineEnding(original)).toBe("crlf")
    expect(serializeText("changed\ntext\n", content(original, "mixed"))).toBe("changed\r\ntext\r\n")
  })
})
