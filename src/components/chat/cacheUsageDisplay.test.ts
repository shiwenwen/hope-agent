import { test, expect } from "vitest"

import { formatCacheUsageDisplay } from "./cacheUsageDisplay.ts"

test("formats cache usage with explicit write and hit labels while keeping lightning", () => {
  expect(
    formatCacheUsageDisplay({
      created: 0,
      read: 0,
      writeLabel: "写入",
      hitLabel: "命中",
    }),
  ).toBe("写入 0 / ⚡命中 0")
})

test("keeps compact k formatting for large cache token counts", () => {
  expect(
    formatCacheUsageDisplay({
      created: 1234,
      read: 5678,
      writeLabel: "Written",
      hitLabel: "Hit",
    }),
  ).toBe("Written 1.2k / ⚡Hit 5.7k")
})
