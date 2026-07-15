import { describe, expect, it } from "vitest"

import { shouldSubmitProjectInstructions } from "./projectInstructionsDraft"

describe("shouldSubmitProjectInstructions", () => {
  it("does not submit the loaded AGENTS.md draft for metadata-only edits", () => {
    expect(
      shouldSubmitProjectInstructions("# Rules\n", "# Rules\n", "/tmp/project", "/tmp/project"),
    ).toBe(false)
  })

  it("submits when AGENTS.md content changes", () => {
    expect(
      shouldSubmitProjectInstructions("# Updated\n", "# Rules\n", "/tmp/project", "/tmp/project"),
    ).toBe(true)
  })

  it("submits when the project working directory changes", () => {
    expect(
      shouldSubmitProjectInstructions("# Rules\n", "# Rules\n", "/tmp/other", "/tmp/project"),
    ).toBe(true)
  })

  it("ignores surrounding working-directory whitespace", () => {
    expect(shouldSubmitProjectInstructions("", "", " /tmp/project ", "/tmp/project")).toBe(false)
  })
})
