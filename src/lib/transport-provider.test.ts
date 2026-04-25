/**
 * Contract: transport-provider exposes `getTransport()` only.
 * A bare `transport` named export would silently destructure to undefined
 * and break callers like `({ transport }) => transport.listen(...)`.
 * The `@ts-expect-error` below trips `tsc --noEmit` if anyone reintroduces it.
 */

import { test, expect } from "vitest"

import type * as TransportProvider from "./transport-provider.ts"

/* eslint-disable @typescript-eslint/no-unused-vars -- type-level contract assertions */
type _HasGetTransport = TransportProvider["getTransport"]
// @ts-expect-error contract: no bare `transport` export
type _NoBareTransport = TransportProvider["transport"]
/* eslint-enable @typescript-eslint/no-unused-vars */

test("transport-provider exposes getTransport, not a bare `transport` export", async () => {
  const mod = (await import("./transport-provider.ts")) as Record<
    string,
    unknown
  >
  expect(typeof mod.getTransport).toBe("function")
  expect("transport" in mod).toBe(false)
})
