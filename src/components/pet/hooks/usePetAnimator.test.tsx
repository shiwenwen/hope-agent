// @vitest-environment jsdom

import { act, cleanup, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"
import {
  actionForStatus,
  lookTargetForPointer,
  usePetAnimator,
  type PetAction,
  type PetLookTarget,
} from "./usePetAnimator"

function Harness({
  action,
  onComplete,
  lookTarget = null,
}: {
  action: PetAction
  onComplete: (action: PetAction) => void
  lookTarget?: PetLookTarget
}) {
  const animation = usePetAnimator(action, onComplete, lookTarget)
  return <span data-testid="frame">{`${animation.row}:${animation.frame}`}</span>
}

beforeEach(() => {
  vi.useFakeTimers()
  vi.stubGlobal(
    "matchMedia",
    vi.fn(() => ({
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })),
  )
})

afterEach(() => {
  cleanup()
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe("usePetAnimator", () => {
  test("maps business statuses to the official Codex action rows", () => {
    expect(actionForStatus("blocked")).toBe("failed")
    expect(actionForStatus("needs_input")).toBe("waiting")
    expect(actionForStatus("running")).toBe("running")
    expect(actionForStatus("ready")).toBe("review")
  })

  test("maps pointer vectors clockwise across the two v2 look rows", () => {
    const rect = { left: 0, top: 0, width: 100, height: 100 }
    expect(lookTargetForPointer(50, 0, rect)).toBe(0)
    expect(lookTargetForPointer(100, 50, rect)).toBe(4)
    expect(lookTargetForPointer(50, 100, rect)).toBe(8)
    expect(lookTargetForPointer(0, 50, rect)).toBe(12)
    expect(lookTargetForPointer(50, 50, rect)).toBe("neutral")

    const onComplete = vi.fn()
    const { rerender } = render(<Harness action="idle" lookTarget={0} onComplete={onComplete} />)
    expect(screen.getByTestId("frame")).toHaveTextContent("9:0")
    rerender(<Harness action="idle" lookTarget={8} onComplete={onComplete} />)
    expect(screen.getByTestId("frame")).toHaveTextContent("10:0")
    rerender(<Harness action="idle" lookTarget="neutral" onComplete={onComplete} />)
    expect(screen.getByTestId("frame")).toHaveTextContent("0:6")
  })

  test("keeps the idle base frame static for v2 look targets under reduced motion", () => {
    vi.stubGlobal(
      "matchMedia",
      vi.fn(() => ({
        matches: true,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      })),
    )
    const onComplete = vi.fn()
    const { rerender } = render(<Harness action="idle" lookTarget={4} onComplete={onComplete} />)

    expect(screen.getByTestId("frame")).toHaveTextContent("0:0")
    rerender(<Harness action="idle" lookTarget={12} onComplete={onComplete} />)
    expect(screen.getByTestId("frame")).toHaveTextContent("0:0")
  })

  test("completes wave only after three full four-frame cycles", () => {
    const onComplete = vi.fn()
    render(<Harness action="wave" onComplete={onComplete} />)

    expect(screen.getByTestId("frame")).toHaveTextContent("3:0")
    act(() => vi.advanceTimersByTime(700))
    expect(screen.getByTestId("frame")).toHaveTextContent("3:0")
    expect(onComplete).not.toHaveBeenCalled()

    act(() => vi.advanceTimersByTime(700))
    expect(screen.getByTestId("frame")).toHaveTextContent("3:0")
    expect(onComplete).not.toHaveBeenCalled()

    act(() => vi.advanceTimersByTime(699))
    expect(onComplete).not.toHaveBeenCalled()

    act(() => vi.advanceTimersByTime(1))
    expect(onComplete).toHaveBeenCalledOnce()
    expect(onComplete).toHaveBeenCalledWith("wave")
  })
})
