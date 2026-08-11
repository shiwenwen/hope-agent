import { describe, expect, it } from "vitest"
import { expandMentionsToAttachments, resolveMentionWorkingDir } from "./expandMentions"
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

  it("attaches an extensionless root file selected from a project-inherited workspace", () => {
    const raw = "@Dockerfile"
    const workingDir = resolveMentionWorkingDir({
      targetSessionId: "session-1",
      activeSessionId: "session-1",
      sessionWorkingDir: null,
      draftWorkingDir: null,
      mentionWorkingDir: "/workspace/hope-agent-website",
    })
    const binding: ComposerMentionBinding = {
      id: "file-dockerfile",
      kind: "file",
      targetId: "Dockerfile",
      displayLabel: "Dockerfile",
      raw,
      start: 0,
      end: raw.length,
      origin: "first_party_composer_gesture",
    }

    expect(expandMentionsToAttachments(`${raw} 都写了什么`, workingDir, [binding])).toEqual([
      {
        name: "Dockerfile",
        mime_type: "text/plain",
        source: "mention",
        file_path: "/workspace/hope-agent-website/Dockerfile",
      },
    ])
  })

  it("does not borrow the active project workspace for a cross-session send", () => {
    expect(
      resolveMentionWorkingDir({
        targetSessionId: "session-2",
        activeSessionId: "session-1",
        sessionWorkingDir: null,
        draftWorkingDir: null,
        mentionWorkingDir: "/workspace/project-1",
      }),
    ).toBeNull()
  })

  it("uses the project workspace for a not-yet-materialized project chat", () => {
    expect(
      resolveMentionWorkingDir({
        targetSessionId: null,
        activeSessionId: null,
        sessionWorkingDir: null,
        draftWorkingDir: null,
        mentionWorkingDir: "/workspace/project-draft",
      }),
    ).toBe("/workspace/project-draft")
  })

  it("prefers a session working-directory override to the inherited project root", () => {
    expect(
      resolveMentionWorkingDir({
        targetSessionId: "session-1",
        activeSessionId: "session-1",
        sessionWorkingDir: "/workspace/session-override",
        draftWorkingDir: null,
        mentionWorkingDir: "/workspace/project-root",
      }),
    ).toBe("/workspace/session-override")
  })
})
