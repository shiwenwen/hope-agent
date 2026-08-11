import { describe, expect, it } from "vitest"
import { expandMentionsToAttachments } from "./expandMentions"
import type { ComposerMentionBinding } from "../mentions/typedMentions"

describe("expandMentionsToAttachments", () => {
  it("does not read a manually typed or pasted @path without a typed binding", () => {
    expect(expandMentionsToAttachments("read @secret.txt", "/workspace", [])).toEqual([])
  })

  it("attaches the exact file selected by the structured picker", () => {
    const raw = "@src/main.rs"
    const input = `review ${raw}`
    const binding: ComposerMentionBinding = {
      id: "file-1",
      kind: "file",
      targetId: "src/main.rs",
      displayLabel: "src/main.rs",
      raw,
      start: 7,
      end: 7 + raw.length,
      origin: "first_party_composer_gesture",
    }

    expect(expandMentionsToAttachments(input, "/workspace", [binding])).toEqual([
      {
        name: "main.rs",
        mime_type: "text/plain",
        source: "mention",
        file_path: "/workspace/src/main.rs",
      },
    ])
  })
})
