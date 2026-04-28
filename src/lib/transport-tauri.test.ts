import { afterEach, beforeEach, expect, test, vi } from "vitest"

const mocks = vi.hoisted(() => ({
  listen: vi.fn(),
  invoke: vi.fn(),
  convertFileSrc: vi.fn((path: string) => `asset://${path}`),
}))

vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.listen,
}))

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
  convertFileSrc: mocks.convertFileSrc,
  Channel: class {
    onmessage = () => {}
  },
}))

import { TauriTransport } from "./transport-tauri"

let warnSpy: ReturnType<typeof vi.spyOn>

beforeEach(() => {
  mocks.listen.mockReset()
  mocks.invoke.mockReset()
  mocks.convertFileSrc.mockClear()
  warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {})
})

afterEach(() => {
  warnSpy.mockRestore()
})

test("TauriTransport listen cleanup is idempotent after registration", async () => {
  const rawUnlisten = vi.fn()
  mocks.listen.mockResolvedValue(rawUnlisten)

  const off = new TauriTransport().listen("config:changed", vi.fn())
  await Promise.resolve()

  off()
  off()

  expect(rawUnlisten).toHaveBeenCalledTimes(1)
})

test("TauriTransport listen cleanup is idempotent before registration resolves", async () => {
  const rawUnlisten = vi.fn()
  let resolveListen: (fn: () => void) => void = () => {}
  mocks.listen.mockReturnValue(new Promise((resolve) => {
    resolveListen = resolve
  }))

  const off = new TauriTransport().listen("config:changed", vi.fn())

  off()
  off()
  resolveListen(rawUnlisten)
  await Promise.resolve()

  expect(rawUnlisten).toHaveBeenCalledTimes(1)
})
