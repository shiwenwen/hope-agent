// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { DockerSetupHint } from "./DockerSetupHint"

const openExternalUrlMock = vi.hoisted(() => vi.fn())

vi.mock("@/lib/openExternalUrl", () => ({
  openExternalUrl: openExternalUrlMock,
}))

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

describe("DockerSetupHint", () => {
  beforeEach(() => openExternalUrlMock.mockReset())
  afterEach(cleanup)

  it("shows a permission diagnostic instead of an install action", () => {
    render(
      <DockerSetupHint
        status={{
          installed: true,
          running: false,
          connectionError: "permission_denied",
          containerized: true,
        }}
      />,
    )

    expect(screen.getByText("settings.dockerSetupPermissionDenied")).toBeTruthy()
    expect(screen.queryByText("Docker Engine")).toBeNull()
  })

  it("warns when a container deployment selects a bind-mount mode", () => {
    const { rerender } = render(
      <DockerSetupHint
        status={{
          installed: true,
          running: false,
          connectionError: "socket_missing",
          containerized: true,
          isolatedModeOnly: true,
        }}
        sandboxMode="workspace"
      />,
    )

    expect(screen.getByText("settings.dockerSetupContainerIsolatedOnly")).toBeTruthy()
    expect(screen.queryByText("settings.dockerSetupSocketMissingContainer")).toBeNull()

    rerender(
      <DockerSetupHint
        status={{
          installed: true,
          running: false,
          connectionError: "socket_missing",
          containerized: true,
          isolatedModeOnly: true,
        }}
        sandboxMode="isolated"
      />,
    )
    expect(screen.queryByText("settings.dockerSetupContainerIsolatedOnly")).toBeNull()
    expect(screen.getByText("settings.dockerSetupSocketMissingContainer")).toBeTruthy()
  })

  it("opens install documentation in the current web client", () => {
    render(<DockerSetupHint status={{ installed: false, running: false, hostOs: "linux" }} />)

    fireEvent.click(screen.getByText("Docker Engine"))
    expect(openExternalUrlMock).toHaveBeenCalledWith("https://docs.docker.com/engine/install/")
  })
})
