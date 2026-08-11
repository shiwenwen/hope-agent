// @vitest-environment jsdom

import { cleanup, fireEvent, render, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import MentionComposerInput from "./MentionComposerInput"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

vi.mock("@/components/chat/files/useFileResource", () => ({
  useFileResource: () => ({ primary: "open", run: vi.fn() }),
}))

const originalRangeGetClientRects = Range.prototype.getClientRects

beforeEach(() => {
  // CodeMirror measures text after document changes. jsdom exposes Range but
  // omits this layout API, so provide the empty geometry its zero-sized DOM
  // already represents.
  Object.defineProperty(Range.prototype, "getClientRects", {
    configurable: true,
    value: () => [] as unknown as DOMRectList,
  })
})

afterEach(() => {
  cleanup()
  if (originalRangeGetClientRects) {
    Object.defineProperty(Range.prototype, "getClientRects", {
      configurable: true,
      value: originalRangeGetClientRects,
    })
  } else {
    delete (Range.prototype as Partial<Range>).getClientRects
  }
})

describe("MentionComposerInput", () => {
  it("keeps handled Shift+Enter inside CodeMirror", async () => {
    const onChange = vi.fn()
    const onOuterKeyDown = vi.fn()
    const view = render(
      <div onKeyDown={onOuterKeyDown}>
        <MentionComposerInput
          value="first line"
          placeholder="Ask anything"
          workingDir={null}
          fileEnabled={false}
          noteEnabled={false}
          onChange={onChange}
          onKeyDown={vi.fn()}
          onPaste={vi.fn()}
          onSelectionChange={vi.fn()}
        />
      </div>,
    )

    const editor = view.container.querySelector<HTMLElement>(".cm-content")
    expect(editor).not.toBeNull()
    fireEvent.keyDown(editor!, { key: "Enter", code: "Enter", shiftKey: true })

    await waitFor(() => expect(onChange).toHaveBeenCalledWith("\nfirst line"))
    expect(onOuterKeyDown).not.toHaveBeenCalled()
  })
})
