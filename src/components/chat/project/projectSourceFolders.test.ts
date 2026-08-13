import { describe, expect, it } from "vitest"

import {
  projectSourceFoldersForSession,
  projectRootFromInstructionsPath,
  promoteProjectSourceFolder,
  resolveProjectFormPrimaryDir,
} from "./projectSourceFolders"

describe("projectRootFromInstructionsPath", () => {
  it("extracts POSIX and Windows managed workspace roots", () => {
    expect(projectRootFromInstructionsPath("/data/projects/project-1/workspace/AGENTS.md")).toBe(
      "/data/projects/project-1/workspace",
    )
    expect(projectRootFromInstructionsPath("C:\\Hope\\project-1\\workspace\\AGENTS.md")).toBe(
      "C:\\Hope\\project-1\\workspace",
    )
    expect(projectRootFromInstructionsPath("README.md")).toBeNull()
  })
})

describe("resolveProjectFormPrimaryDir", () => {
  it("does not resurrect a persisted explicit root after the form clears it", () => {
    expect(resolveProjectFormPrimaryDir("", "/workspace/old", null)).toBeNull()
    const defaultWorkspace = resolveProjectFormPrimaryDir(
      "",
      "/workspace/old",
      "/data/projects/project-1/workspace",
    )
    expect(defaultWorkspace).toBe("/data/projects/project-1/workspace")
    expect(
      promoteProjectSourceFolder("/workspace/new", defaultWorkspace ?? "", ["/workspace/new"]),
    ).toEqual({
      workingDir: "/workspace/new",
      linkedDirs: ["/data/projects/project-1/workspace"],
    })
  })

  it("keeps the persisted effective root until the form explicitly clears it", () => {
    expect(resolveProjectFormPrimaryDir("", "/data/projects/project-1/workspace")).toBe(
      "/data/projects/project-1/workspace",
    )
    expect(resolveProjectFormPrimaryDir(" /workspace/new ", "/workspace/old", null)).toBe(
      "/workspace/new",
    )
  })
})

describe("promoteProjectSourceFolder", () => {
  it("preserves an implicit default workspace when promoting a linked folder", () => {
    expect(
      promoteProjectSourceFolder("/workspace/linked", "/data/projects/project-1/workspace", [
        "/workspace/linked",
        "/workspace/docs",
      ]),
    ).toEqual({
      workingDir: "/workspace/linked",
      linkedDirs: ["/data/projects/project-1/workspace", "/workspace/docs"],
    })
  })

  it("does not duplicate a former primary already present in the linked roots", () => {
    expect(
      promoteProjectSourceFolder("/workspace/linked", "/workspace/main", [
        "/workspace/main",
        "/workspace/linked",
      ]),
    ).toEqual({
      workingDir: "/workspace/linked",
      linkedDirs: ["/workspace/main"],
    })
  })
})

describe("projectSourceFoldersForSession", () => {
  it("appends the Project primary after stable linked-root indices when cwd is overridden", () => {
    expect(
      projectSourceFoldersForSession(
        ["/workspace/api", "/workspace/docs"],
        "/workspace/session-override",
        "/workspace/project-primary",
      ),
    ).toEqual(["/workspace/api", "/workspace/docs", "/workspace/project-primary"])
  })

  it("does not duplicate the primary when it is already active or linked", () => {
    expect(
      projectSourceFoldersForSession(["/workspace/api"], "/workspace/main", "/workspace/main"),
    ).toEqual(["/workspace/api"])
    expect(
      projectSourceFoldersForSession(["/workspace/main"], "/workspace/other", "/workspace/main"),
    ).toEqual(["/workspace/main"])
    expect(projectSourceFoldersForSession(["/workspace/api"], null, "/workspace/main")).toEqual([
      "/workspace/api",
    ])
  })
})
