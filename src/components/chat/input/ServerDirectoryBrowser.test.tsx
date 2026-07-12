// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import ServerDirectoryBrowser from "./ServerDirectoryBrowser"

const transportMock = vi.hoisted(() => ({
  listServerDirectory: vi.fn<(path?: string) => Promise<{ path: string; entries: never[]; truncated: boolean }>>(),
}))

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (_key: string, fallback?: string) => fallback ?? _key,
  }),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
}))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe("ServerDirectoryBrowser", () => {
  it("loads a manually typed path before selecting the current directory", async () => {
    transportMock.listServerDirectory.mockImplementation((path?: string) =>
      Promise.resolve({ path: path ?? "/", entries: [], truncated: false }),
    )
    const onSelect = vi.fn()

    render(
      <ServerDirectoryBrowser
        open
        onOpenChange={() => {}}
        initialPath="/"
        onSelect={onSelect}
      />,
    )

    const input = await screen.findByPlaceholderText("chat.workingDir.pathPlaceholder")
    fireEvent.change(input, { target: { value: "/repo" } })

    expect(screen.getByRole("button", { name: "跳转到路径" })).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "chat.workingDir.selectCurrent" }))

    await waitFor(() => {
      expect(transportMock.listServerDirectory).toHaveBeenLastCalledWith("/repo")
      expect(onSelect).toHaveBeenCalledWith("/repo")
    })
  })
})
