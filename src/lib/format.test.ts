import { test, expect } from "vitest"
import { formatBytes, formatBytesFromMb, formatGbFromMb } from "./format"

test("formats bytes with automatic units", () => {
  expect(formatBytes(0)).toBe("0 B")
  expect(formatBytes(512)).toBe("512 B")
  expect(formatBytes(1536)).toBe("1.5 KB")
  expect(formatBytes(2 * 1024 * 1024)).toBe("2.0 MB")
  expect(formatBytes(3.25 * 1024 * 1024 * 1024)).toBe("3.3 GB")
})

test("supports forced and capped units", () => {
  expect(formatBytes(1536, { unit: "KB" })).toBe("1.5 KB")
  expect(formatBytes(3 * 1024 * 1024, { maxUnit: "KB" })).toBe("3072.0 KB")
  expect(formatBytes(8 * 1024 * 1024, { unit: "MB", fractionDigits: 0 })).toBe("8 MB")
})

test("formats megabyte source values for model sizes", () => {
  expect(formatBytesFromMb(512)).toBe("512 MB")
  expect(formatBytesFromMb(4608)).toBe("4.5 GB")
})

test("formats megabyte source values as bare gigabyte numbers", () => {
  expect(formatGbFromMb(16384)).toBe("16.0")
  expect(formatGbFromMb(1536)).toBe("1.5")
})
