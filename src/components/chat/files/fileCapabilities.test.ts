import { describe, expect, it } from "vitest"

import type { FileRuntime, WorkspaceAccess } from "@/lib/transport"
import { resolveFileCapabilities } from "./fileCapabilities"
import { createDraftAttachment, type FileTarget } from "./types"

const LOCAL_DESKTOP: FileRuntime = {
  workspaceHost: "local",
  openMode: "system",
  canReveal: true,
}
const REMOTE_DESKTOP: FileRuntime = {
  workspaceHost: "remote",
  openMode: "browser",
  canReveal: false,
}
const WEB: FileRuntime = {
  workspaceHost: "remote",
  openMode: "browser",
  canReveal: false,
}

const ACCESS: Record<WorkspaceAccess["writeState"], WorkspaceAccess> = {
  enabled: { readable: true, writeState: "enabled" },
  remote_writes_disabled: { readable: true, writeState: "remote_writes_disabled" },
  scope_read_only: { readable: true, writeState: "scope_read_only" },
  project_archived: { readable: true, writeState: "project_archived" },
}

function targets(): FileTarget[] {
  return [
    {
      kind: "clientDraft",
      draft: createDraftAttachment(
        new File(["draft"], "draft.md", { type: "text/markdown" }),
        "paste",
      ),
      previewId: "draft-preview",
    },
    {
      kind: "workspace",
      scope: "session",
      scopeId: "session-a",
      relPath: "README.md",
      name: "README.md",
      mime: "text/markdown",
    },
    {
      kind: "sessionPath",
      sessionId: "session-a",
      path: "/tmp/report.pdf",
      name: "report.pdf",
      mime: "application/pdf",
    },
    {
      kind: "media",
      item: {
        url: "/api/attachments/session-a/report.pdf",
        name: "report.pdf",
        mimeType: "application/pdf",
        sizeBytes: 100,
        kind: "file",
      },
    },
    { kind: "knowledgeNote", kbId: "kb-a", path: "notes/idea.md" },
    {
      kind: "artifact",
      artifactId: "artifact-a",
      name: "Quarterly report.html",
      projectPath: "/tmp/artifact-a",
    },
  ]
}

describe("resolveFileCapabilities", () => {
  it.each([
    ["local desktop", LOCAL_DESKTOP],
    ["remote desktop", REMOTE_DESKTOP],
    ["web", WEB],
  ] as const)("keeps all six targets previewable in %s", (_label, runtime) => {
    for (const target of targets()) {
      const capabilities = resolveFileCapabilities(target, runtime, ACCESS.enabled)
      expect(capabilities.preview.state, target.kind).toBe("enabled")
      expect(capabilities.open.state, target.kind).toBe("enabled")
      expect(capabilities.download.state, target.kind).toBe("enabled")
    }
  })

  it("only exposes reveal where files live on the local desktop", () => {
    for (const runtime of [REMOTE_DESKTOP, WEB]) {
      for (const target of targets()) {
        expect(resolveFileCapabilities(target, runtime, ACCESS.enabled).reveal.state).toBe(
          "disabled",
        )
      }
    }

    for (const target of targets().filter((item) => item.kind !== "clientDraft")) {
      expect(resolveFileCapabilities(target, LOCAL_DESKTOP, ACCESS.enabled).reveal.state).toBe(
        "enabled",
      )
    }
  })

  it.each([
    ["enabled", "enabled"],
    ["remote_writes_disabled", "guided"],
    ["scope_read_only", "disabled"],
    ["project_archived", "disabled"],
  ] as const)("maps workspace write state %s to %s", (writeState, expected) => {
    const workspace = targets().find((target) => target.kind === "workspace")!
    const capabilities = resolveFileCapabilities(workspace, REMOTE_DESKTOP, ACCESS[writeState])
    for (const action of [
      "edit",
      "rename",
      "delete",
      "createFile",
      "createFolder",
      "upload",
      "saveAs",
    ] as const) {
      expect(capabilities[action].state, action).toBe(expected)
    }
  })

  it("lets intrinsic read-only reasons win over the remote-write guide", () => {
    const workspace = targets().find((target) => target.kind === "workspace")!
    expect(resolveFileCapabilities(workspace, REMOTE_DESKTOP, ACCESS.scope_read_only).edit).toEqual(
      { state: "disabled", reason: "scope_read_only" },
    )
    expect(
      resolveFileCapabilities(workspace, REMOTE_DESKTOP, ACCESS.project_archived).edit,
    ).toEqual({ state: "disabled", reason: "project_archived" })
  })

  it("lets intrinsic read-only reasons win over file type and size limits", () => {
    const oversizedBinary: FileTarget = {
      kind: "workspace",
      scope: "project",
      scopeId: "project-a",
      relPath: "archive.bin",
      name: "archive.bin",
      sizeBytes: 99,
    }

    expect(
      resolveFileCapabilities(oversizedBinary, REMOTE_DESKTOP, ACCESS.project_archived, 1).edit,
    ).toEqual({ state: "disabled", reason: "project_archived" })
    expect(
      resolveFileCapabilities(oversizedBinary, REMOTE_DESKTOP, ACCESS.scope_read_only, 1).edit,
    ).toEqual({ state: "disabled", reason: "scope_read_only" })
  })

  it("edits a client text draft in memory but never reveals its source file", () => {
    const draft = targets().find((target) => target.kind === "clientDraft")!
    const capabilities = resolveFileCapabilities(draft, LOCAL_DESKTOP)
    expect(capabilities.edit.state).toBe("enabled")
    expect(capabilities.remove.state).toBe("enabled")
    expect(capabilities.reveal).toEqual({ state: "disabled", reason: "reveal_unavailable" })
  })

  it("keeps directory mutations available without treating directories as previewable files", () => {
    const directory: FileTarget = {
      kind: "workspace",
      scope: "session",
      scopeId: "session-a",
      relPath: "docs",
      name: "docs",
      isDirectory: true,
    }
    const capabilities = resolveFileCapabilities(directory, LOCAL_DESKTOP, ACCESS.enabled)

    expect(capabilities.preview.state).toBe("disabled")
    expect(capabilities.open.state).toBe("disabled")
    expect(capabilities.createFile.state).toBe("enabled")
    expect(capabilities.createFolder.state).toBe("enabled")
    expect(capabilities.upload.state).toBe("enabled")
    expect(capabilities.rename.state).toBe("enabled")
    expect(capabilities.delete.state).toBe("enabled")
  })
})
