/// <reference types="node" />

import assert from "node:assert/strict"
import { test } from "node:test"

import { formatCacheUsageDisplay } from "./cacheUsageDisplay.ts"

test("formats cache usage with explicit write and hit labels while keeping lightning", () => {
  assert.equal(
    formatCacheUsageDisplay({
      created: 0,
      read: 0,
      writeLabel: "写入",
      hitLabel: "命中",
    }),
    "写入 0 / ⚡命中 0",
  )
})

test("keeps compact k formatting for large cache token counts", () => {
  assert.equal(
    formatCacheUsageDisplay({
      created: 1234,
      read: 5678,
      writeLabel: "Written",
      hitLabel: "Hit",
    }),
    "Written 1.2k / ⚡Hit 5.7k",
  )
})
