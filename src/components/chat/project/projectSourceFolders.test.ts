import { describe, expect, it } from "vitest"

import { promoteProjectSourceFolder } from "./projectSourceFolders"

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
