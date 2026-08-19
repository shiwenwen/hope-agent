// @vitest-environment jsdom

import { cleanup, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, test, vi } from "vitest"
import BrowserPanel from "./BrowserPanel"
import CanvasPanel from "./CanvasPanel"
import { FileBrowserPanel } from "./FileBrowserPanel"
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
  artifactPreviewUrl: vi.fn(async (id: string) => `/api/canvas/projects/${id}/index.html`),
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
  useTransportRevision: () => 0,
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

function renderPanel(ui: React.ReactNode) {
  return render(<TooltipProvider>{ui}</TooltipProvider>)
}

describe("docked panel contract", () => {
  test("BrowserPanel keeps refresh but delegates float and close to the workbench", () => {
    renderPanel(<BrowserPanel onClose={() => {}} onFloat={() => {}} />)

    expect(screen.getByRole("button", { name: "chat.browserPanel.refresh" })).toBeTruthy()
    expect(screen.queryByRole("button", { name: "chat.controlPanel.floatWindow" })).toBeNull()
    expect(screen.queryByRole("button", { name: "chat.browserPanel.close" })).toBeNull()
  })

  test("FileBrowserPanel renders its browser body inside the shell", () => {
    renderPanel(
      <FileBrowserPanel scope="session" scopeId="s1" rootPath="/repo" sessionId="s1" visible onClose={() => {}} />,
    )

    expect(screen.getByText("File browser body")).toBeTruthy()
  })

  test("CanvasPanel is interactive immediately when the shared dock requests mount animation", async () => {
    const requestFrame = vi.spyOn(window, "requestAnimationFrame").mockImplementation(() => 1)
    const { container } = renderPanel(<CanvasPanel currentSessionId="s1" visible animateOnMount />)

    await waitFor(() => expect(screen.getByText("Canvas Preview")).toBeTruthy())
    const shell = container.firstElementChild as HTMLElement
    expect(shell.getAttribute("aria-hidden")).toBeNull()
    expect(shell.hasAttribute("inert")).toBe(false)
    requestFrame.mockRestore()
  })
})
