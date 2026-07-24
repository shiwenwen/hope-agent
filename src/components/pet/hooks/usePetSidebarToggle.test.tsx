// @vitest-environment jsdom

import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"

const mocks = vi.hoisted(() => ({
  call: vi.fn(),
  listeners: new Map<string, (payload: unknown) => void>(),
}))

vi.mock("@/lib/transport", () => ({ isTauriMode: () => true }))
vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => ({
    call: mocks.call,
    listen: (event: string, listener: (payload: unknown) => void) => {
      mocks.listeners.set(event, listener)
      return () => mocks.listeners.delete(event)
    },
  }),
}))

import { usePetSidebarToggle } from "./usePetSidebarToggle"

let backendEnabled = false

beforeEach(() => {
  backendEnabled = false
  mocks.call.mockReset()
  mocks.listeners.clear()
  mocks.call.mockImplementation((command: string, args?: { enabled?: boolean }) => {
    if (command === "get_pet_config_cmd") {
      return Promise.resolve({ enabled: backendEnabled, selectedPetRef: "builtin:hope-default" })
    }
    if (command === "pet_set_enabled_cmd") {
      backendEnabled = Boolean(args?.enabled)
      return Promise.resolve({ enabled: backendEnabled, selectedPetRef: "builtin:hope-default" })
    }
    return Promise.reject(new Error(`Unexpected command: ${command}`))
  })
})

afterEach(cleanup)

describe("usePetSidebarToggle", () => {
  test("toggles the pet and follows config changes from other surfaces", async () => {
    const { result } = renderHook(() => usePetSidebarToggle())

    await waitFor(() => expect(result.current.ready).toBe(true))
    expect(result.current.enabled).toBe(false)

    await act(async () => {
      expect(await result.current.toggle()).toBe(true)
    })
    expect(mocks.call).toHaveBeenCalledWith("pet_set_enabled_cmd", {
      enabled: true,
      source: "sidebar",
    })
    expect(result.current.enabled).toBe(true)

    backendEnabled = false
    act(() => mocks.listeners.get("pet:config_changed")?.({ source: "pet-window" }))
    await waitFor(() => expect(result.current.enabled).toBe(false))
  })
})
