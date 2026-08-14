// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, test, vi } from "vitest"

import { ArtifactSelectionIframe } from "./ArtifactViewer"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

describe("ArtifactSelectionIframe", () => {
  test("uses an inert document while a managed preview URL is unresolved", () => {
    render(<ArtifactSelectionIframe src="" title="Loading report" />)
    expect(screen.getByTitle("Loading report").getAttribute("src")).toBe("about:blank")
  })

  test("accepts only bounded selections from its live iframe and stages rather than sends", async () => {
    const onQuoteSelection = vi.fn()
    render(
      <ArtifactSelectionIframe
        src="https://server.test/api/canvas/projects/a/index.html"
        title="Report"
        onQuoteSelection={onQuoteSelection}
      />,
    )

    const iframe = screen.getByTitle("Report") as HTMLIFrameElement
    vi.spyOn(iframe, "getBoundingClientRect").mockReturnValue({
      x: 100,
      y: 80,
      left: 100,
      top: 80,
      right: 600,
      bottom: 480,
      width: 500,
      height: 400,
      toJSON: () => ({}),
    })
    const frameWindow = iframe.contentWindow
    expect(frameWindow).not.toBeNull()
    const postMessage = vi.spyOn(frameWindow!, "postMessage")
    fireEvent.load(iframe)
    const activation = postMessage.mock.calls.at(-1)?.[0] as {
      token: string
      version: number
    }

    const send = (data: Record<string, unknown>, source: MessageEventSource = frameWindow!) => {
      act(() => {
        window.dispatchEvent(new MessageEvent("message", { data, source }))
      })
    }
    const base = {
      type: "hope_artifact_text_selection",
      version: activation.version,
      token: activation.token,
      text: "Selected evidence",
      rect: { left: 20, top: 40, right: 180, bottom: 64 },
    }

    send(base, window)
    expect(screen.queryByText("fileBrowser.quoteToChat")).toBeNull()

    send({ ...base, truncated: true })
    expect(screen.queryByText("fileBrowser.quoteToChat")).toBeNull()

    send(base)
    const quote = await screen.findByText("fileBrowser.quoteToChat")
    fireEvent.click(quote)
    await waitFor(() => {
      expect(onQuoteSelection).toHaveBeenCalledWith({ text: "Selected evidence" })
    })
    expect(onQuoteSelection).toHaveBeenCalledTimes(1)
    expect(iframe.getAttribute("sandbox")).toBe("allow-scripts")
  })
})
