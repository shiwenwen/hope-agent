import { describe, expect, it } from "vitest"

import {
  projectSourceFoldersForSession,
  promoteProjectSourceFolder,
} from "./projectSourceFolders"

describe("promoteProjectSourceFolder", () => {
  it("preserves an implicit default workspace when promoting a linked folder", () => {
    expect(
      promoteProjectSourceFolder(
        "/workspace/linked",
        "/data/projects/project-1/workspace",
        ["/workspace/linked", "/workspace/docs"],
      ),
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
    expect(projectSourceFoldersForSession(["/workspace/api"], "/workspace/main", "/workspace/main"))
      .toEqual(["/workspace/api"])
    expect(projectSourceFoldersForSession(["/workspace/main"], "/workspace/other", "/workspace/main"))
      .toEqual(["/workspace/main"])
    expect(projectSourceFoldersForSession(["/workspace/api"], null, "/workspace/main")).toEqual([
      "/workspace/api",
    ])
  })
})
