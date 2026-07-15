import { describe, expect, it, vi } from "vitest"

import type { Transport } from "@/lib/transport"
import { fileResourceAdapterFor, type WorkspaceFileOperations } from "./fileResourceAdapter"
import type { FileTarget } from "./types"

const target: Extract<FileTarget, { kind: "workspace" }> = {
  kind: "workspace",
  scope: "session",
  scopeId: "session-a",
  relPath: "docs",
  name: "docs",
  isDirectory: true,
}

function setup() {
  const operations: WorkspaceFileOperations = {
    createFile: vi.fn(async () => true),
    createFolder: vi.fn(async () => true),
    rename: vi.fn(async () => true),
    remove: vi.fn(async () => true),
    uploadInto: vi.fn(async () => true),
    saveAs: vi.fn(async () => ({ status: "saved" })),
  }
  const transport = {
    fileRuntime: () => ({ workspaceHost: "local", openMode: "system", canReveal: true }),
  } as Transport
  return { adapter: fileResourceAdapterFor(target), operations, transport }
}

describe("workspace file resource adapter", () => {
  it("dispatches every workspace mutation through the shared operations contract", async () => {
    const { adapter, operations, transport } = setup()
    const context = { transport, workspaceOperations: operations }
    const file = new File(["upload"], "upload.txt", { type: "text/plain" })

    await expect(adapter.run(target, "createFile", context, { name: "note.md" })).resolves.toBe(
      true,
    )
    await expect(adapter.run(target, "createFolder", context, { name: "nested" })).resolves.toBe(
      true,
    )
    await expect(adapter.run(target, "rename", context, { toPath: "renamed" })).resolves.toBe(true)
    await expect(adapter.run(target, "delete", context)).resolves.toBe(true)
    await expect(adapter.run(target, "upload", context, { files: [file] })).resolves.toBe(true)
    await expect(
      adapter.run(target, "saveAs", context, { path: "copy.md", content: "copy" }),
    ).resolves.toBe(true)

    expect(operations.createFile).toHaveBeenCalledWith("docs", "note.md")
    expect(operations.createFolder).toHaveBeenCalledWith("docs", "nested")
    expect(operations.rename).toHaveBeenCalledWith("docs", "renamed")
    expect(operations.remove).toHaveBeenCalledWith("docs", true)
    expect(operations.uploadInto).toHaveBeenCalledWith("docs", [file])
    expect(operations.saveAs).toHaveBeenCalledWith("copy.md", "copy")
  })
})

describe("Artifact file resource adapter", () => {
  it("routes open and formatted downloads through the transport abstraction", async () => {
    const artifact: Extract<FileTarget, { kind: "artifact" }> = {
      kind: "artifact",
      artifactId: "artifact-a",
      name: "Report.html",
      projectPath: "/managed/artifact-a",
    }
    const transport = {
      openArtifact: vi.fn(async () => undefined),
      revealArtifact: vi.fn(async () => undefined),
      downloadArtifact: vi.fn(async () => ({
        filename: "report.pdf",
        receipt: { status: "ready" },
      })),
    } as unknown as Transport
    const adapter = fileResourceAdapterFor(artifact)

    await expect(adapter.run(artifact, "open", { transport })).resolves.toBe(true)
    await expect(adapter.run(artifact, "reveal", { transport })).resolves.toBe(true)
    await expect(
      adapter.run(artifact, "download", { transport }, { artifactFormat: "pdf" }),
    ).resolves.toBe(true)

    expect(transport.openArtifact).toHaveBeenCalledWith("artifact-a", "/managed/artifact-a")
    expect(transport.revealArtifact).toHaveBeenCalledWith("artifact-a", "/managed/artifact-a")
    expect(transport.downloadArtifact).toHaveBeenCalledWith("artifact-a", "pdf")
  })
})
