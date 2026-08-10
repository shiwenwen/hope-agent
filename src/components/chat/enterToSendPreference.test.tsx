// @vitest-environment jsdom

import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, test, vi } from "vitest"

import { useEnterToSendPreference } from "./enterToSendPreference"

const transportState = vi.hoisted(() => {
  const createTransport = () => ({
    call: vi.fn<(command: string) => Promise<unknown>>(),
    listen: vi.fn<(event: string, listener: (payload: unknown) => void) => () => void>(),
  })
  return {
    createTransport,
    current: createTransport(),
    revision: 0,
  }
})

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportState.current,
  useTransportRevision: () => transportState.revision,
}))

afterEach(() => {
  cleanup()
  transportState.revision = 0
  transportState.current = transportState.createTransport()
})

describe("useEnterToSendPreference", () => {
  test("reloads and re-subscribes when the active transport changes", async () => {
    const first = transportState.current
    const firstUnlisten = vi.fn()
    first.call.mockResolvedValue({ enterToSend: true })
    first.listen.mockReturnValue(firstUnlisten)

    const hook = renderHook(() => useEnterToSendPreference())
    await waitFor(() => expect(hook.result.current).toEqual({ enabled: true, ready: true }))

    const second = transportState.createTransport()
    second.call.mockResolvedValue({ enterToSend: false })
    second.listen.mockReturnValue(vi.fn())
    transportState.current = second
    transportState.revision = 1
    hook.rerender()

    expect(firstUnlisten).toHaveBeenCalledTimes(1)
    expect(hook.result.current.ready).toBe(false)
    await waitFor(() => expect(hook.result.current).toEqual({ enabled: false, ready: true }))
    expect(second.call).toHaveBeenCalledWith("get_user_config")
    expect(second.listen).toHaveBeenCalledWith("config:changed", expect.any(Function))
  })

  test("reloads a user config event without pausing for unrelated config changes", async () => {
    const transport = transportState.current
    let configListener: ((payload: unknown) => void) | undefined
    let resolveReload: ((value: { enterToSend: boolean }) => void) | undefined
    transport.call.mockResolvedValueOnce({ enterToSend: true }).mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveReload = resolve
        }),
    )
    transport.listen.mockImplementation((_event, listener) => {
      configListener = listener
      return vi.fn()
    })

    const hook = renderHook(() => useEnterToSendPreference())
    await waitFor(() => expect(hook.result.current).toEqual({ enabled: true, ready: true }))

    act(() => configListener?.({ category: "app" }))
    expect(hook.result.current.ready).toBe(true)
    expect(transport.call).toHaveBeenCalledTimes(1)

    act(() => configListener?.({ category: "user" }))
    expect(hook.result.current.ready).toBe(false)
    expect(transport.call).toHaveBeenCalledTimes(2)

    await act(async () => resolveReload?.({ enterToSend: false }))
    expect(hook.result.current).toEqual({ enabled: false, ready: true })
  })
})
