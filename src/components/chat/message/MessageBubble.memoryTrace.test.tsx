// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"

import { TooltipProvider } from "@/components/ui/tooltip"
import type { Message } from "@/types/chat"
import MessageBubble from "./MessageBubble"

const toastMock = vi.hoisted(() => ({
  error: vi.fn(),
  success: vi.fn(),
}))

vi.mock("sonner", () => ({
  toast: toastMock,
}))

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: () => {} },
  useTranslation: () => ({
    t: (key: string, fallbackOrOptions?: string | Record<string, unknown>) => {
      let text =
        typeof fallbackOrOptions === "string"
          ? fallbackOrOptions
          : typeof fallbackOrOptions?.defaultValue === "string"
            ? fallbackOrOptions.defaultValue
            : key
      if (fallbackOrOptions && typeof fallbackOrOptions === "object") {
        for (const [name, value] of Object.entries(fallbackOrOptions)) {
          text = text.replaceAll(`{{${name}}}`, String(value))
        }
      }
      return text
    },
  }),
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
  toastMock.error.mockClear()
  toastMock.success.mockClear()
})

function renderMessageBubble(msg: Message) {
  return render(
    <TooltipProvider>
      <MessageBubble
        msg={msg}
        index={0}
        isLast={false}
        loading={false}
        executionState={null}
        agents={[]}
        isHovered={false}
        onHover={() => {}}
        onContextMenu={() => {}}
        isCopied={false}
        onCopy={() => {}}
        sessionId="session-1"
      />
    </TooltipProvider>,
  )
}

describe("MessageBubble memory trace actions", () => {
  test("does not show SQLite edit actions for the project auto-memory index", async () => {
    renderMessageBubble({
      role: "assistant",
      content: "answer",
      usedMemoryRefs: [
        {
          kind: "memory",
          id: "project-auto-memory-index:project-1",
          sourceType: "project_auto_memory_index",
          scope: "project:project-1",
          origin: "project_auto_memory",
          role: "injected",
          preview: "2 project memory topics indexed",
        },
      ],
    })

    fireEvent.click(screen.getByRole("button", { name: /used memory/i }))

    expect(await screen.findByText("2 project memory topics indexed")).toBeTruthy()
    expect(screen.queryByRole("button", { name: "Quick edit memory" })).toBeNull()
    expect(screen.queryByRole("button", { name: "Do not use this memory" })).toBeNull()
  })

  test("shows a redacted toast when copying diagnostics fails", async () => {
    const writeText = vi
      .fn()
      .mockRejectedValue(
        new Error("clipboard denied token=copy-secret Authorization: Bearer bearer-secret"),
      )
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    })

    renderMessageBubble({
      role: "assistant",
      content: "answer",
      usedMemoryRefs: [
        {
          kind: "memory",
          id: "42",
          origin: "active_memory",
          role: "selected",
          preview: "User prefers concise answers.",
          score: 0.91,
        },
      ],
      retrievalPlanner: {
        status: "used",
        totalRefs: 1,
        layers: [
          {
            layer: "active_memory",
            status: "used",
            refCount: 1,
            selectedCount: 1,
            latencyMs: 12,
          },
        ],
      },
    })

    fireEvent.click(screen.getByRole("button", { name: /used memory/i }))
    fireEvent.click(await screen.findByRole("button", { name: /copy diagnostics/i }))

    await waitFor(() => {
      expect(toastMock.error).toHaveBeenCalledWith("Failed to copy memory diagnostics", {
        description: "Details: clipboard denied token=[redacted] Authorization: Bearer [redacted]",
      })
    })
    expect(writeText).toHaveBeenCalledTimes(1)
    expect(writeText.mock.calls[0]?.[0]).toContain("User prefers concise answers.")
  })
})
