// @vitest-environment jsdom

import { act, cleanup, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"
import { usePetAnimator, type PetAction } from "./usePetAnimator"

function Harness({ action, onComplete }: { action: PetAction; onComplete: (action: PetAction) => void }) {
  const animation = usePetAnimator(action, onComplete)
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
