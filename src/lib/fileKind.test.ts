import { describe, expect, it } from "vitest"
import { fileKindOf, shikiLang } from "./fileKind"

describe("fileKind", () => {
  it("treats common TypeScript module variants as code with TypeScript highlighting", () => {
    expect(fileKindOf("vite.config.mts")).toBe("code")
    expect(fileKindOf("schema.cts")).toBe("code")
    expect(shikiLang("vite.config.mts")).toBe("typescript")
    expect(shikiLang("schema.cts")).toBe("typescript")
  })

  it("uses explicit metadata language when a file name has no useful extension", () => {
    expect(fileKindOf("generated", null, "typescript")).toBe("code")
    expect(shikiLang("generated", "ts")).toBe("typescript")
  })

  it("highlights extensionless code filenames with their conventional grammar", () => {
    expect(fileKindOf("Dockerfile")).toBe("code")
    expect(shikiLang("Dockerfile")).toBe("dockerfile")
  })
})
