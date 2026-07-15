// @vitest-environment jsdom

import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import {
  disposeCronUnreadStore,
  initCronUnreadStore,
  useCronUnreadStore,
} from "./useCronUnreadStore"

const listeners = new Map<string, (raw: unknown) => void>()
const calls: string[] = []
let cronUnreadTotal = 0

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => ({
    call: async (command: string) => {
      calls.push(command)
      if (command === "cron_unread_total") return cronUnreadTotal
      return null
    },
    listen: (event: string, callback: (raw: unknown) => void) => {
      listeners.set(event, callback)
      return () => listeners.delete(event)
    },
  }),
}))

afterEach(() => {
  cleanup()
  disposeCronUnreadStore()
  listeners.clear()
  calls.length = 0
  cronUnreadTotal = 0
  vi.clearAllMocks()
})

describe("useCronUnreadStore invalidation", () => {
  it("reconciles an unscoped batch read event but ignores an unrelated explicit domain", async () => {
    cronUnreadTotal = 2
    initCronUnreadStore()
    const { result } = renderHook(() => useCronUnreadStore())

    await waitFor(() => expect(result.current.cronUnreadCount).toBe(2))
    calls.length = 0

    cronUnreadTotal = 0
    act(() => listeners.get("session:unread_changed")?.({ domain: null }))

    await waitFor(() => expect(result.current.cronUnreadCount).toBe(0))
    expect(calls).toEqual(["cron_unread_total"])

    calls.length = 0
    act(() => listeners.get("session:unread_changed")?.({ domain: "regular" }))
    await Promise.resolve()
    expect(calls).toEqual([])
  })
})
