import { describe, expect, it } from "vitest"

import { quoteReferencePath, resolveProjectFileQuoteTarget } from "./fileQuoteTarget"

describe("linked-root file quotes", () => {
  it("uses an absolute linked-root path for model and history references", () => {
    expect(
      quoteReferencePath({
        path: "src/index.ts",
        name: "index.ts",
        startLine: 1,
        endLine: 2,
        content: "export {}",
        projectRoot: { index: 1, path: "/repos/shared" },
      }),
    ).toBe("/repos/shared/src/index.ts")
  })

  it("restores the exact current linked root and fails closed after it changes", () => {
    const quote = { path: "src/index.ts", projectRoot: { index: 1, path: "/repos/shared" } }
    expect(resolveProjectFileQuoteTarget(quote, ["/repos/api", "/repos/shared"])).toEqual({
      path: "src/index.ts",
      projectRoot: { index: 1, path: "/repos/shared" },
      valid: true,
    })
    expect(resolveProjectFileQuoteTarget(quote, ["/repos/api", "/repos/replaced"])).toEqual({
      path: "src/index.ts",
      projectRoot: null,
      valid: false,
    })
  })

  it("recovers an older persisted absolute quote without rebinding prefixes", () => {
    expect(
      resolveProjectFileQuoteTarget(
        { path: "/repos/shared/src/index.ts" },
        ["/repos/share", "/repos/shared"],
      ),
    ).toEqual({
      path: "src/index.ts",
      projectRoot: { index: 1, path: "/repos/shared" },
      valid: true,
    })
  })
})
