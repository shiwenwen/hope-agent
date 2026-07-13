// @vitest-environment jsdom

import { afterEach, describe, expect, test } from "vitest"

import { installFocusVisibilityTracker } from "./focus-visibility"

let uninstall: (() => void) | null = null

function install() {
  uninstall = installFocusVisibilityTracker()
}

afterEach(() => {
  uninstall?.()
  uninstall = null
  document.body.replaceChildren()
  delete document.documentElement.dataset.inputModality
  delete document.documentElement.dataset.focusIndicators
})

describe("focus visibility input modality", () => {
  test("starts quiet and switches between keyboard and pointer input", () => {
    install()
    expect(document.documentElement.dataset.inputModality).toBe("pointer")
    expect(document.documentElement.dataset.focusIndicators).toBe("auto")

    const button = document.createElement("button")
    document.body.append(button)
    button.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true }))
    expect(document.documentElement.dataset.inputModality).toBe("keyboard")

    button.dispatchEvent(new Event("pointerdown", { bubbles: true }))
    expect(document.documentElement.dataset.inputModality).toBe("pointer")
  })

  test("typing in a pointer-focused editor does not paint a focus ring", () => {
    install()
    const input = document.createElement("input")
    document.body.append(input)

    input.dispatchEvent(new Event("pointerdown", { bubbles: true }))
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "a", bubbles: true }))
    expect(document.documentElement.dataset.inputModality).toBe("pointer")

    input.dispatchEvent(new KeyboardEvent("keydown", { key: "é", altKey: true, bubbles: true }))
    expect(document.documentElement.dataset.inputModality).toBe("pointer")

    const altGraphInput = new KeyboardEvent("keydown", {
      key: "@",
      altKey: true,
      ctrlKey: true,
      bubbles: true,
    })
    Object.defineProperty(altGraphInput, "getModifierState", {
      value: (modifier: string) => modifier === "AltGraph",
    })
    input.dispatchEvent(altGraphInput)
    expect(document.documentElement.dataset.inputModality).toBe("pointer")

    input.dispatchEvent(new KeyboardEvent("keydown", { key: "f", ctrlKey: true, bubbles: true }))
    expect(document.documentElement.dataset.inputModality).toBe("keyboard")
  })

  test("keyboard interaction on non-editable controls enables the indicator", () => {
    install()
    const radio = document.createElement("input")
    radio.type = "radio"
    document.body.append(radio)

    radio.dispatchEvent(new Event("pointerdown", { bubbles: true }))
    radio.dispatchEvent(new KeyboardEvent("keydown", { key: " ", bubbles: true }))
    expect(document.documentElement.dataset.inputModality).toBe("keyboard")

    radio.dispatchEvent(new Event("pointerdown", { bubbles: true }))
    radio.focus()
    expect(document.documentElement.dataset.inputModality).toBe("pointer")
  })
})
