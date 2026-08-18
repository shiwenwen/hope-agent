// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"
import { cleanup, render, screen } from "@testing-library/react"

import { TooltipProvider } from "@/components/ui/tooltip"
import type { ChatDisplayMode, Message } from "@/types/chat"
import MessageBubble from "./MessageBubble"

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: () => {} },
  useTranslation: () => ({ t: (key: string) => key }),
}))

vi.mock("./MessageContent", () => ({
  AssistantContentBlocks: ({ msg }: { msg: Message }) => (
    <div data-testid="assistant-content">{msg.content}</div>
  ),
}))

vi.mock("./MessageUrlPreviews", () => ({
  default: () => null,
}))

vi.mock("@/components/common/ProviderIcon", () => ({
  default: () => <span data-testid="provider-icon" />,
}))

beforeEach(() => {
  vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback: FrameRequestCallback) => {
    callback(0)
    return 0
  })
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => {})
})

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

function renderBubble(options: { displayMode: ChatDisplayMode; hideActionBar?: boolean }) {
  const msg: Message = {
    role: "assistant",
    content: "answer",
    model: "gpt-5.1",
  } as Message

  return render(
    <TooltipProvider>
      <MessageBubble
        msg={msg}
        index={0}
        isLast={false}
        loading={false}
        executionState={null}
        agents={[]}
        isHovered
        onHover={() => {}}
        onContextMenu={() => {}}
        isCopied={false}
        onCopy={() => {}}
        sessionId="session-1"
        displayMode={options.displayMode}
        hideActionBar={options.hideActionBar}
      />
    </TooltipProvider>,
  )
}

describe("MessageBubble action bar", () => {
  for (const displayMode of ["bubble", "timeline"] as const) {
    test(`renders a single action bar per bubble (${displayMode})`, () => {
      renderBubble({ displayMode })
      expect(screen.getAllByTestId("message-action-bar")).toHaveLength(1)
    })

    test(`drops the action bar for folded process steps (${displayMode})`, () => {
      renderBubble({ displayMode, hideActionBar: true })
      expect(screen.queryByTestId("message-action-bar")).toBeNull()
    })
  }
})
