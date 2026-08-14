// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"

import { SelectableDomPreview } from "./SelectableDomPreview"
import { selectionWithLineRange } from "./selectionLineRange"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallback?: string) => fallback ?? key,
  }),
}))

function pointerEvent(type: string) {
  const event = new Event(type, { bubbles: true, cancelable: true })
  Object.defineProperties(event, {
    isPrimary: { value: true },
    button: { value: 0 },
  })
  return event
}

function selectText(node: Text, start: number, end: number) {
  const range = document.createRange()
  range.setStart(node, start)
  range.setEnd(node, end)
  Object.defineProperties(range, {
    getClientRects: {
      value: () => [new DOMRect(40, 80, 60, 18)],
    },
    getBoundingClientRect: {
      value: () => new DOMRect(40, 80, 60, 18),
    },
  })
  const selection = window.getSelection()
  selection?.removeAllRanges()
  selection?.addRange(range)
  return selection
}

beforeEach(() => {
  vi.useFakeTimers()
})

afterEach(() => {
  cleanup()
  window.getSelection()?.removeAllRanges()
  vi.useRealTimers()
})

describe("selectionWithLineRange", () => {
  test("maps a rendered Markdown selection back to literal source lines", () => {
    expect(selectionWithLineRange("second line", "# Title\n\nfirst\nsecond line\nlast")).toEqual({
      text: "second line",
      startLine: 4,
      endLine: 4,
    })
  })

  test("does not invent lines for Office, formatted, duplicated, or unmapped selections", () => {
    expect(selectionWithLineRange("cell value")).toMatchObject({ startLine: 0, endLine: 0 })
    expect(
      selectionWithLineRange("hello bold world", "intro\nhello **bold** world\noutro"),
    ).toMatchObject({ startLine: 0, endLine: 0 })
    expect(selectionWithLineRange("repeat", "repeat\nrepeat")).toMatchObject({
      startLine: 0,
      endLine: 0,
    })
    expect(selectionWithLineRange("aaa", "aaaa")).toMatchObject({
      startLine: 0,
      endLine: 0,
    })
    expect(selectionWithLineRange("render-only", "unrelated source")).toMatchObject({
      startLine: 0,
      endLine: 0,
    })
  })
})

describe("SelectableDomPreview", () => {
  test("waits for pointer-up, then quotes without clearing the live selection", () => {
    const onQuote = vi.fn()
    render(
      <SelectableDomPreview sourceText={"# Hello world"} onQuote={onQuote}>
        <p data-testid="content">Hello world</p>
      </SelectableDomPreview>,
    )
    const content = screen.getByTestId("content")
    const textNode = content.firstChild as Text

    act(() => {
      content.dispatchEvent(pointerEvent("pointerdown"))
      selectText(textNode, 0, 5)
      document.dispatchEvent(new Event("selectionchange"))
      vi.advanceTimersByTime(150)
    })
    expect(screen.queryByRole("button", { name: "Quote to chat" })).not.toBeInTheDocument()

    act(() => {
      content.dispatchEvent(pointerEvent("pointerup"))
      vi.runOnlyPendingTimers()
    })
    const quote = screen.getByRole("button", { name: "Quote to chat", hidden: true })
    const down = pointerEvent("pointerdown")
    fireEvent(quote, down)
    expect(down.defaultPrevented).toBe(true)
    expect(window.getSelection()?.toString()).toBe("Hello")

    fireEvent.click(quote)
    expect(onQuote).toHaveBeenCalledWith({
      text: "Hello",
      startLine: 1,
      endLine: 1,
    })
  })
})
