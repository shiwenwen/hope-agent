// @vitest-environment jsdom

import { afterEach, describe, expect, test, vi } from "vitest"
import { resolvePetInactiveHoverTarget } from "./usePetInactivePointer"

const originalElementFromPoint = Object.getOwnPropertyDescriptor(document, "elementFromPoint")

afterEach(() => {
  document.body.replaceChildren()
  vi.restoreAllMocks()
  if (originalElementFromPoint) {
    Object.defineProperty(document, "elementFromPoint", originalElementFromPoint)
  } else {
    Reflect.deleteProperty(document, "elementFromPoint")
  }
})

describe("resolvePetInactiveHoverTarget", () => {
  test("maps native coordinates to the live bubble action", () => {
    const bubble = document.createElement("div")
    bubble.dataset.petActivityId = "session-1"
    const stop = document.createElement("button")
    stop.dataset.petHoverAction = "stop"
    const icon = document.createElementNS("http://www.w3.org/2000/svg", "svg")
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path")
    icon.append(path)
    stop.append(icon)
    bubble.append(stop)
    document.body.append(bubble)
    Object.defineProperty(document, "elementFromPoint", {
      configurable: true,
      value: vi.fn(() => path),
    })

    expect(resolvePetInactiveHoverTarget(20, 30)).toEqual({
      activityId: "session-1",
      action: "stop",
      pet: false,
    })
  })

  test("maps the pet sprite separately and clears empty transparent space", () => {
    const sprite = document.createElement("button")
    sprite.dataset.petSprite = ""
    document.body.append(sprite)
    const hitTest = vi.fn<() => Element | null>(() => sprite)
    Object.defineProperty(document, "elementFromPoint", {
      configurable: true,
      value: hitTest,
    })

    expect(resolvePetInactiveHoverTarget(10, 12)).toEqual({
      activityId: null,
      action: null,
      pet: true,
    })
    hitTest.mockReturnValue(null)
    expect(resolvePetInactiveHoverTarget(100, 100)).toEqual({
      activityId: null,
      action: null,
      pet: false,
    })
  })
})
