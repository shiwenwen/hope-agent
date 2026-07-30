// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import ModelRecoveryBanner from "./ModelRecoveryBanner"

const transportMock = vi.hoisted(() => ({
  call: vi.fn(),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
}))

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) => key,
  }),
}))

afterEach(() => {
  cleanup()
  vi.useRealTimers()
  transportMock.call.mockReset()
})

describe("ModelRecoveryBanner", () => {
  it("counts down and applies an exact model-switch recovery action", async () => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date("2026-07-30T00:00:00Z"))
    transportMock.call.mockResolvedValue({ applied: true })

    render(
      <ModelRecoveryBanner
        sessionId="session-1"
        event={{
          type: "model_retry",
          reason: "timeout",
          delay_ms: 5_000,
          recovery_id: "recovery-1",
          can_switch_model: true,
        }}
      />,
    )

    expect(screen.getByText("5s")).toBeTruthy()
    act(() => vi.advanceTimersByTime(1_200))
    expect(screen.getByText("4s")).toBeTruthy()

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "chat.recoverySwitchModel" }))
      await Promise.resolve()
    })
    expect(transportMock.call).toHaveBeenCalledWith("control_model_recovery", {
      sessionId: "session-1",
      recoveryId: "recovery-1",
      action: "switch_model",
    })
  })

  it("offers only immediate start for a whole-chain recovery wait", () => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date("2026-07-30T00:00:00Z"))

    render(
      <ModelRecoveryBanner
        sessionId="session-1"
        event={{
          type: "model_chain_retry",
          delay_ms: 4_000,
          recovery_id: "recovery-2",
          can_switch_model: false,
        }}
      />,
    )

    expect(screen.getByRole("button", { name: "chat.recoveryStartNow" })).toBeTruthy()
    expect(screen.queryByRole("button", { name: "chat.recoverySwitchModel" })).toBeNull()
  })
})
