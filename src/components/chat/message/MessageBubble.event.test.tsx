// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, test, vi } from "vitest"

import type { Message } from "@/types/chat"
import MessageBubble from "./MessageBubble"
import { subscribeChatFocus } from "../chatFocus"
import { parseSessionMessages } from "../chatUtils"

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: () => {} },
  useTranslation: () => ({ t: (key: string) => key }),
}))

vi.mock("@/components/common/ProviderIcon", () => ({
  default: () => <span data-testid="provider-icon" />,
}))

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

function renderEventMessage(
  content: Record<string, unknown>,
  onConfigureVisionBridge?: () => void,
) {
  const msg: Message = {
    role: "event",
    content: JSON.stringify(content),
  }

  return render(
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
      onConfigureVisionBridge={onConfigureVisionBridge}
    />,
  )
}

describe("MessageBubble persisted events", () => {
  test.each([undefined, "parent-chat"])(
    "renders persisted cross-session provenance and opens its source (side parent: %s)",
    (sideParentSessionId) => {
      const [msg] = parseSessionMessages([
        {
          id: 42,
          sessionId: "destination",
          role: "user",
          content: "Please review the changes.",
          timestamp: "2026-08-31T12:00:00Z",
          attachmentsMeta: JSON.stringify({
            session_message: { sessionId: "source-chat", title: "Review", sideParentSessionId },
          }),
        },
      ])
      const onFocus = vi.fn()
      const unsubscribe = subscribeChatFocus(onFocus)
      try {
        render(
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
            sessionId="destination"
          />,
        )
        fireEvent.click(screen.getByRole("button", { name: "chat.crossSession.receivedFrom" }))
        expect(onFocus).toHaveBeenCalledWith(
          sideParentSessionId
            ? { sessionId: sideParentSessionId, sideSessionId: "source-chat" }
            : { sessionId: "source-chat" },
        )
        expect(screen.getByText("Please review the changes.")).toBeTruthy()
      } finally {
        unsubscribe()
      }
    },
  )

  test("opens the vision bridge settings from an ignored-image notice", () => {
    const onConfigureVisionBridge = vi.fn()
    renderEventMessage(
      {
        type: "vision_auto_disabled",
        provider_name: "Gateway",
        model_id: "text-only-model",
      },
      onConfigureVisionBridge,
    )

    fireEvent.click(screen.getByRole("button", { name: "chat.visionBridgeConfigureAction" }))
    expect(onConfigureVisionBridge).toHaveBeenCalledTimes(1)
  })

  test("renders a persisted model fallback with the fallback banner", () => {
    const payload = {
      type: "model_fallback",
      model: "OpenAI / gpt-5.1",
      from_model: "Codex / gpt-5.2-codex",
      provider_id: "openai",
      model_id: "gpt-5.1",
      reason: "auth_error",
      attempt: 2,
      total: 3,
      error: "authentication failed",
    }

    const { container } = renderEventMessage(payload)

    expect(screen.getByText("chat.fallbackTitle")).toBeTruthy()
    expect(screen.getByText("gpt-5.2-codex")).toBeTruthy()
    expect(screen.getByText("gpt-5.1")).toBeTruthy()
    expect(screen.getByText("2/3")).toBeTruthy()
    expect(container.textContent).not.toContain(JSON.stringify(payload))

    fireEvent.click(screen.getByRole("button"))
    expect(screen.getByText("authentication failed")).toBeTruthy()
  })
})
