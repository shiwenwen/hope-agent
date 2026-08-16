import { describe, expect, it } from "vitest"

import { environmentInsetWidth } from "./environmentInset"

const base = { lane: 348, contentMaxWidth: 880, minContentWidth: 420 }

function contentBox(available: number, inset: number) {
  const left = 16
  const box = available - left - inset
  const width = Math.min(base.contentMaxWidth, box)
  return { left: left + (box - width) / 2, right: left + (box + width) / 2 }
}

describe("environmentInsetWidth", () => {
  it("leaves a wide layout alone — the centred column already clears the card", () => {
    expect(environmentInsetWidth({ ...base, available: 2200 })).toBe(0)
    expect(contentBox(2200, 0).right).toBeLessThanOrEqual(2200 - base.lane)
  })

  it("reserves the lane as soon as the centred column would collide", () => {
    expect(environmentInsetWidth({ ...base, available: 1500 })).toBe(348)
    expect(contentBox(1500, 0).right).toBeGreaterThan(1500 - base.lane)
    expect(contentBox(1500, base.lane).right).toBeLessThanOrEqual(1500 - base.lane)
  })

  it("shifts before it shrinks", () => {
    // Wide enough to keep the full 880 column, so only its position moves.
    const inset = environmentInsetWidth({ ...base, available: 1400 })
    const before = contentBox(1400, 0)
    const after = contentBox(1400, inset)
    expect(after.right - after.left).toBe(880)
    expect(after.left).toBeLessThan(before.left)
  })

  it("shrinks once shifting runs out of room", () => {
    const after = contentBox(1000, environmentInsetWidth({ ...base, available: 1000 }))
    expect(after.right - after.left).toBeLessThan(880)
    expect(after.right).toBeLessThanOrEqual(1000 - base.lane)
  })

  it("gives up and lets the card overlap once the lane leaves too little", () => {
    expect(environmentInsetWidth({ ...base, available: 740 })).toBe(0)
    expect(environmentInsetWidth({ ...base, available: 400 })).toBe(0)
  })

  it("is inert without a measured column", () => {
    expect(environmentInsetWidth({ ...base, available: 0 })).toBe(0)
    expect(environmentInsetWidth({ ...base, available: 1500, lane: 0 })).toBe(0)
  })
})
