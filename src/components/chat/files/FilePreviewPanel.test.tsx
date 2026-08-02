// @vitest-environment jsdom

import { cleanup, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, expect, test, vi } from "vitest"

import { TooltipProvider } from "@/components/ui/tooltip"
import FilePreviewPanel from "./FilePreviewPanel"

const state = vi.hoisted(() => ({
  revision: 0,
  artifactPreviewUrl: vi.fn(),
}))

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

vi.mock("@/lib/transport-provider", () => ({
  useTransport: () => ({ artifactPreviewUrl: state.artifactPreviewUrl }),
  useTransportRevision: () => state.revision,
}))

vi.mock("@/lib/filesystemConfig", () => ({
  useFilesystemConfig: () => ({
    config: { maxTextPreviewMb: 5, maxDocumentPreviewMb: 20 },
  }),
  MEBIBYTE_BYTES: 1024 * 1024,
}))

vi.mock("./useFileResource", () => ({
  useFileResource: () => ({
    run: vi.fn(),
    isLocal: false,
    capabilities: {
      open: { state: "enabled" },
      download: { state: "enabled" },
      edit: { state: "disabled" },
    },
  }),
}))

beforeEach(() => {
  state.revision = 0
  state.artifactPreviewUrl.mockReset()
  state.artifactPreviewUrl.mockImplementation(
    async () => `https://agent.example/api/resource/ticket-${state.revision}/canvas/index.html`,
  )
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

test("reloads an open scoped preview when transport credentials rotate", async () => {
  const target = { kind: "artifact", artifactId: "artifact-1", name: "Report.html" } as const
  const view = () => (
    <TooltipProvider>
      <FilePreviewPanel target={target} onClose={vi.fn()} />
    </TooltipProvider>
  )
  const rendered = render(view())

  await waitFor(() => {
    expect(screen.getByTitle("Report.html").getAttribute("src")).toContain("ticket-0")
  })

  state.revision = 1
  rendered.rerender(view())

  await waitFor(() => {
    expect(screen.getByTitle("Report.html").getAttribute("src")).toContain("ticket-1")
  })
  expect(state.artifactPreviewUrl).toHaveBeenCalledTimes(2)
})
