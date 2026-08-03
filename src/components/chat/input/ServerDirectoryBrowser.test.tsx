// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { TRANSPORT_EVENT_RESYNC_REQUIRED } from "@/lib/transport"
import ServerDirectoryBrowser from "./ServerDirectoryBrowser"

const transportMock = vi.hoisted(() => ({
  listServerDirectory:
    vi.fn<(path?: string) => Promise<{ path: string; entries: never[]; truncated: boolean }>>(),
  createDirectory:
    vi.fn<(path: string) => Promise<{ path: string; entries: never[]; truncated: boolean }>>(),
  call: vi.fn(),
  listen: vi.fn<(event: string, handler: (payload: unknown) => void) => () => void>(),
  fileRuntime: vi.fn(),
}))

const transportListeners = new Map<string, Set<(payload: unknown) => void>>()

function emitTransportEvent(event: string, payload: unknown = undefined) {
  for (const handler of transportListeners.get(event) ?? []) handler(payload)
}

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallback?: string | { defaultValue?: string }) =>
      typeof fallback === "string" ? fallback : (fallback?.defaultValue ?? key),
  }),
}))

vi.mock("@/lib/transport-provider", () => ({
  useTransport: () => transportMock,
}))

beforeEach(() => {
  vi.resetAllMocks()
  transportListeners.clear()
  transportMock.call.mockResolvedValue({ allowRemoteWrites: true })
  transportMock.listen.mockImplementation((event, handler) => {
    const handlers = transportListeners.get(event) ?? new Set()
    handlers.add(handler)
    transportListeners.set(event, handlers)
    return () => handlers.delete(handler)
  })
  transportMock.fileRuntime.mockReturnValue({ workspaceHost: "remote" })
})

afterEach(() => {
  cleanup()
})

describe("ServerDirectoryBrowser", () => {
  it("loads a manually typed path before selecting the current directory", async () => {
    transportMock.listServerDirectory.mockImplementation((path?: string) =>
      Promise.resolve({ path: path ?? "/", entries: [], truncated: false }),
    )
    const onSelect = vi.fn()

    render(
      <ServerDirectoryBrowser open onOpenChange={() => {}} initialPath="/" onSelect={onSelect} />,
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

  it("returns to the account home when the home shortcut is clicked", async () => {
    transportMock.listServerDirectory.mockImplementation((path?: string) =>
      Promise.resolve({ path: path ?? "/home/hope", entries: [], truncated: false }),
    )

    render(
      <ServerDirectoryBrowser
        open
        onOpenChange={() => {}}
        initialPath="/srv/projects"
        onSelect={() => {}}
      />,
    )

    await screen.findByDisplayValue("/srv/projects")
    fireEvent.click(screen.getByRole("button", { name: "chat.workingDir.home" }))

    await waitFor(() => {
      expect(transportMock.listServerDirectory).toHaveBeenLastCalledWith(undefined)
      expect(screen.getByDisplayValue("/home/hope")).toBeTruthy()
    })
  })

  it("creates a folder under the currently listed writable directory", async () => {
    transportMock.listServerDirectory.mockResolvedValue({
      path: "/home/hope",
      entries: [],
      truncated: false,
    })
    transportMock.createDirectory.mockResolvedValue({
      path: "/home/hope/project-a",
      entries: [],
      truncated: false,
    })
    const onSelect = vi.fn()

    render(<ServerDirectoryBrowser open allowCreate onOpenChange={() => {}} onSelect={onSelect} />)

    const folderName = await screen.findByPlaceholderText("Folder name")
    fireEvent.change(folderName, { target: { value: "project-a" } })
    fireEvent.click(screen.getByRole("button", { name: "common.create" }))

    await waitFor(() => {
      expect(transportMock.createDirectory).toHaveBeenCalledWith("/home/hope/project-a")
      expect(onSelect).toHaveBeenCalledWith("/home/hope/project-a")
    })
  })

  it("keeps local desktop creation available without the remote-write opt-in", async () => {
    transportMock.fileRuntime.mockReturnValue({ workspaceHost: "local" })
    transportMock.call.mockResolvedValue({ allowRemoteWrites: false })
    transportMock.listServerDirectory.mockResolvedValue({
      path: "/Users/hope",
      entries: [],
      truncated: false,
    })

    render(<ServerDirectoryBrowser open allowCreate onOpenChange={() => {}} onSelect={() => {}} />)

    expect(await screen.findByPlaceholderText("Folder name")).toBeTruthy()
    expect(transportMock.call).not.toHaveBeenCalled()
  })

  it("guides HTTP users to Server settings before offering remote directory creation", async () => {
    transportMock.call.mockResolvedValue({ allowRemoteWrites: false })
    transportMock.listServerDirectory.mockResolvedValue({
      path: "/home/hope",
      entries: [],
      truncated: false,
    })
    const onOpenChange = vi.fn()
    const navigate = vi.fn()
    window.addEventListener("settings:navigate", navigate)

    render(
      <ServerDirectoryBrowser open allowCreate onOpenChange={onOpenChange} onSelect={() => {}} />,
    )

    const settingsButton = await screen.findByRole("button", {
      name: "fileEditor.goToServerSettings",
    })
    expect(screen.queryByPlaceholderText("Folder name")).toBeNull()
    fireEvent.click(settingsButton)

    expect(onOpenChange).toHaveBeenCalledWith(false)
    expect(navigate).toHaveBeenCalledTimes(1)
    window.removeEventListener("settings:navigate", navigate)
  })

  it("checks remote-write capability only while open and recovers after transport resync", async () => {
    transportMock.call.mockRejectedValueOnce(new Error("temporary disconnect"))
    transportMock.listServerDirectory.mockResolvedValue({
      path: "/home/hope",
      entries: [],
      truncated: false,
    })

    const { rerender } = render(
      <ServerDirectoryBrowser
        open={false}
        allowCreate
        onOpenChange={() => {}}
        onSelect={() => {}}
      />,
    )

    expect(transportMock.call).not.toHaveBeenCalled()

    rerender(
      <ServerDirectoryBrowser open allowCreate onOpenChange={() => {}} onSelect={() => {}} />,
    )

    expect(await screen.findByText("chat.workingDir.remoteWritesUnavailable")).toBeTruthy()
    expect(screen.queryByText("fileEditor.remoteWritesTitle")).toBeNull()

    transportMock.call.mockResolvedValue({ allowRemoteWrites: true })
    emitTransportEvent(TRANSPORT_EVENT_RESYNC_REQUIRED, { reason: "reconnected" })

    expect(await screen.findByPlaceholderText("Folder name")).toBeTruthy()
  })

  it("retries an unavailable remote-write capability check on demand", async () => {
    transportMock.call.mockRejectedValueOnce(new Error("temporary disconnect"))
    transportMock.listServerDirectory.mockResolvedValue({
      path: "/home/hope",
      entries: [],
      truncated: false,
    })

    render(<ServerDirectoryBrowser open allowCreate onOpenChange={() => {}} onSelect={() => {}} />)

    expect(await screen.findByText("chat.workingDir.remoteWritesUnavailable")).toBeTruthy()
    transportMock.call.mockResolvedValue({ allowRemoteWrites: true })
    fireEvent.click(screen.getByRole("button", { name: "common.retry" }))

    expect(await screen.findByPlaceholderText("Folder name")).toBeTruthy()
    expect(transportMock.call).toHaveBeenCalledTimes(2)
  })

  it("turns OS permission failures into an actionable message with technical details", async () => {
    transportMock.listServerDirectory.mockResolvedValue({
      path: "/",
      entries: [],
      truncated: false,
    })
    transportMock.createDirectory.mockRejectedValue(
      new Error("directory is not writable: '/': Permission denied (os error 13)"),
    )

    render(<ServerDirectoryBrowser open allowCreate onOpenChange={() => {}} onSelect={() => {}} />)

    const folderName = await screen.findByPlaceholderText("Folder name")
    fireEvent.change(folderName, { target: { value: "project-a" } })
    fireEvent.click(screen.getByRole("button", { name: "common.create" }))

    expect(await screen.findByText("chat.workingDir.locationNotWritable")).toBeTruthy()
    fireEvent.click(screen.getByText("chat.details"))
    expect(await screen.findByText(/Permission denied \(os error 13\)/)).toBeTruthy()
  })
})
