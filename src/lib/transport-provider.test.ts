/**
 * Contract: transport-provider exposes `getTransport()` only.
 * A bare `transport` named export would silently destructure to undefined
 * and break callers like `({ transport }) => transport.listen(...)`.
 *
 * - The static `import { getTransport }` below trips `tsc` if anyone removes
 *   that named export.
 * - The runtime check ensures no one re-introduces a bare `transport` export.
 */

import { test, expect } from "vitest"
import { getTransport } from "./transport-provider.ts"

test("transport-provider exposes getTransport, not a bare `transport` export", async () => {
  expect(typeof getTransport).toBe("function")

  const mod = (await import("./transport-provider.ts")) as Record<
    string,
    unknown
  >
  expect("transport" in mod).toBe(false)
})
