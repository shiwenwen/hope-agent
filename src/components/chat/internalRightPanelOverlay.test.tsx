// @vitest-environment jsdom

import { cleanup, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, test, vi } from "vitest"
import BrowserPanel from "./BrowserPanel"
import CanvasPanel from "./CanvasPanel"
import { FileBrowserPanel } from "./FileBrowserPanel"
import MacControlPanel from "./MacControlPanel"
import { TeamPanel } from "@/components/team/TeamPanel"
import { TooltipProvider } from "@/components/ui/tooltip"

const transportMock = vi.hoisted(() => ({
  call: vi.fn((name: string) => {
    if (name === "list_canvas_projects_by_session") {
      return Promise.resolve([
        {
          id: "canvas-1",
          title: "Canvas Preview",
          contentType: "html",
          projectPath: "/tmp/canvas-1",
          sessionId: "s1",
        },
      ])
    }
    if (name === "mac_control_capture_frame") return Promise.resolve({})
    return Promise.resolve(null)
  }),
  listen: vi.fn(() => () => {}),
  resolveAssetUrl: vi.fn((path: string) => path),
  artifactPreviewUrl: vi.fn((id: string) => `/api/canvas/projects/${id}/index.html`),
}))

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallback?: string) => (typeof fallback === "string" ? fallback : key),
  }),
  // Needed by src/i18n/i18n.ts, pulled in transitively via chatUtils
  // (PanelActionTimeline reuses its formatDuration).
  initReactI18next: { type: "3rdParty", init: () => {} },
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
  useTransport: () => transportMock,
}))

vi.mock("@/lib/transport", () => ({
  isTauriMode: () => false,
  parsePayload: (raw: unknown) => raw,
}))

vi.mock("@tauri-apps/api/webviewWindow", () => ({
  WebviewWindow: class {
    once() {}
    close() {
      return Promise.resolve()
    }
  },
}))

vi.mock("@tauri-apps/api/window", () => ({
  LogicalSize: class {
    width: number
    height: number

    constructor(width: number, height: number) {
      this.width = width
      this.height = height
    }
  },
  getCurrentWindow: () => ({ setMinSize: vi.fn() }),
}))

vi.mock("./project/file-browser/FileBrowserView", () => ({
  FileBrowserView: () => <div>File browser body</div>,
}))

vi.mock("@/components/team/useTeam", () => ({
  useTeam: () => ({
    team: { id: "team-1", name: "Product Team", status: "running" },
    members: [],
    messages: [],
    tasks: [],
    sendMessage: vi.fn(),
    hasMore: false,
    loadingMore: false,
    loadMoreMessages: vi.fn(),
  }),
}))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

function expectOverlay(container: HTMLElement) {
  const shell = container.firstElementChild
  expect(shell?.className).toContain("fixed")
  expect(shell?.className).toContain("inset-0")
}

function renderPanel(ui: React.ReactNode) {
  return render(<TooltipProvider>{ui}</TooltipProvider>)
}

describe("internal right-panel overlay contract", () => {
  test("BrowserPanel uses the shared fixed overlay surface", () => {
    const { container } = renderPanel(<BrowserPanel overlay onClose={() => {}} />)

    expectOverlay(container)
  })

  test("FileBrowserPanel uses the shared fixed overlay surface", () => {
    const { container } = renderPanel(
      <FileBrowserPanel
        scope="session"
        scopeId="s1"
        rootPath="/repo"
        sessionId="s1"
        visible
        overlay
        panelWidth={480}
        onPanelWidthChange={() => {}}
        onClose={() => {}}
      />,
    )

    expectOverlay(container)
    expect(screen.getByText("File browser body")).toBeTruthy()
  })

  test("MacControlPanel uses the shared fixed overlay surface", () => {
    const { container } = renderPanel(<MacControlPanel overlay onClose={() => {}} />)

    expectOverlay(container)
  })

  test("TeamPanel uses the shared fixed overlay surface", () => {
    const { container } = renderPanel(
      <TeamPanel teamId="team-1" overlay onClose={() => {}} />,
    )

    expectOverlay(container)
  })

  test("CanvasPanel uses the shared fixed overlay surface after restoring a canvas", async () => {
    const { container } = renderPanel(<CanvasPanel currentSessionId="s1" visible overlay />)

    await waitFor(() => expect(screen.getByText("Canvas Preview")).toBeTruthy())
    expectOverlay(container)
  })

  test("CanvasPanel is interactive immediately when the shared dock requests mount animation", async () => {
    const requestFrame = vi
      .spyOn(window, "requestAnimationFrame")
      .mockImplementation(() => 1)
    const { container } = renderPanel(
      <CanvasPanel currentSessionId="s1" visible animateOnMount />,
    )

    await waitFor(() => expect(screen.getByText("Canvas Preview")).toBeTruthy())
    const shell = container.firstElementChild as HTMLElement
    expect(shell.style.width).not.toBe("0px")
    expect(shell.getAttribute("aria-hidden")).toBeNull()
    expect(shell.hasAttribute("inert")).toBe(false)
    requestFrame.mockRestore()
  })
})
