// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import "@testing-library/jest-dom/vitest"
import { afterEach, describe, expect, test, vi } from "vitest"
import { PetBubble } from "./PetBubble"
import type { PetActivity } from "@/types/pet"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { defaultValue?: string }) => options?.defaultValue ?? key,
  }),
}))

const runningActivity: PetActivity = {
  activityId: "session-1",
  status: "running",
  title: "Design the pet interaction",
  titleKind: "session",
  updatedAt: "2026-07-24T00:00:00Z",
  target: { kind: "regular", sessionId: "session-1" },
}

afterEach(cleanup)

describe("PetBubble", () => {
  test("reveals the compact composer only after the reply action is chosen", () => {
    const onExpandReply = vi.fn()
    const { rerender } = render(
      <PetBubble
        activity={runningActivity}
        expanded={false}
        interactionPending={false}
        onOpen={vi.fn()}
        onDismiss={vi.fn()}
        onExpandReply={onExpandReply}
        onCollapseReply={vi.fn()}
        onReply={vi.fn()}
        onStop={vi.fn()}
      />,
    )

    expect(screen.queryByPlaceholderText("Ask anything…")).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole("button", { name: "Reply" }))
    expect(onExpandReply).toHaveBeenCalledOnce()

    rerender(
      <PetBubble
        activity={runningActivity}
        expanded
        interactionPending={false}
        onOpen={vi.fn()}
        onDismiss={vi.fn()}
        onExpandReply={onExpandReply}
        onCollapseReply={vi.fn()}
        onReply={vi.fn()}
        onStop={vi.fn()}
      />,
    )
    expect(screen.getByPlaceholderText("Ask anything…")).toBeInTheDocument()
  })

  test("sends Enter as a queued reply while preserving Shift+Enter for composition", async () => {
    const onReply = vi.fn(async () => "queued" as const)
    render(
      <PetBubble
        activity={runningActivity}
        expanded
        interactionPending={false}
        onOpen={vi.fn()}
        onDismiss={vi.fn()}
        onExpandReply={vi.fn()}
        onCollapseReply={vi.fn()}
        onReply={onReply}
        onStop={vi.fn()}
      />,
    )
    const composer = screen.getByPlaceholderText("Ask anything…")
    fireEvent.change(composer, { target: { value: "Keep the icon compact" } })
    fireEvent.keyDown(composer, { key: "Enter", shiftKey: true })
    expect(onReply).not.toHaveBeenCalled()
    fireEvent.keyDown(composer, { key: "Enter" })

    await waitFor(() => expect(onReply).toHaveBeenCalledWith("Keep the icon compact"))
    expect(await screen.findByText("Reply queued")).toBeInTheDocument()
  })

  test("shows the terminal assistant preview for a ready conversation", () => {
    render(
      <PetBubble
        activity={{
          ...runningActivity,
          status: "ready",
          preview: "The interaction design is ready for review.",
        }}
        expanded={false}
        interactionPending={false}
        onOpen={vi.fn()}
        onDismiss={vi.fn()}
        onExpandReply={vi.fn()}
        onCollapseReply={vi.fn()}
        onReply={vi.fn()}
        onStop={vi.fn()}
      />,
    )
    const preview = screen.getByText("The interaction design is ready for review.")
    expect(preview).toBeInTheDocument()
    expect(preview.parentElement).toHaveClass("line-clamp-2")
    expect(preview.closest("button")).toHaveClass("whitespace-normal")
  })

  test("projects Markdown into readable compact preview text", () => {
    render(
      <PetBubble
        activity={{
          ...runningActivity,
          status: "ready",
          preview:
            "# Update\n\n**Done** with `PetBubble`.\n- [x] Fixed [inactive hover](https://example.com). 快速熟悉结果： ## 项目定位",
        }}
        expanded={false}
        interactionPending={false}
        onOpen={vi.fn()}
        onDismiss={vi.fn()}
        onExpandReply={vi.fn()}
        onCollapseReply={vi.fn()}
        onReply={vi.fn()}
        onStop={vi.fn()}
      />,
    )

    expect(
      screen.getByText("Update Done with PetBubble. Fixed inactive hover. 快速熟悉结果： 项目定位"),
    ).toBeInTheDocument()
    expect(screen.queryByText(/[#*`]/u)).not.toBeInTheDocument()
  })

  test("reports a completed bubble as viewed only after a deliberate hover ends", () => {
    vi.useFakeTimers()
    try {
      const onViewed = vi.fn()
      render(
        <PetBubble
          activity={{ ...runningActivity, status: "ready", boundary: 42, preview: "Finished" }}
          expanded={false}
          interactionPending={false}
          onOpen={vi.fn()}
          onDismiss={vi.fn()}
          onExpandReply={vi.fn()}
          onCollapseReply={vi.fn()}
          onReply={vi.fn()}
          onStop={vi.fn()}
          onViewed={onViewed}
        />,
      )

      const bubble = screen.getByLabelText("Design the pet interaction").parentElement!
      fireEvent.pointerEnter(bubble)
      act(() => vi.advanceTimersByTime(700))
      expect(onViewed).not.toHaveBeenCalled()
      fireEvent.pointerLeave(bubble)
      expect(onViewed).toHaveBeenCalledOnce()
    } finally {
      vi.useRealTimers()
    }
  })

  test("keeps a useful status fallback while a streamed Markdown fence is incomplete", () => {
    render(
      <PetBubble
        activity={runningActivity}
        expanded={false}
        interactionPending={false}
        livePreview="```typescript"
        onOpen={vi.fn()}
        onDismiss={vi.fn()}
        onExpandReply={vi.fn()}
        onCollapseReply={vi.fn()}
        onReply={vi.fn()}
        onStop={vi.fn()}
      />,
    )

    expect(screen.getByText("Thinking")).toBeInTheDocument()
  })

  test("middle-ellipsizes a fallback-style long title so the preview keeps both lines", () => {
    const longTitle =
      "First help me understand every important module, entry point, and development constraint"
    render(
      <PetBubble
        activity={{
          ...runningActivity,
          status: "ready",
          title: longTitle,
          preview: "The repository overview is ready for review.",
        }}
        expanded={false}
        interactionPending={false}
        onOpen={vi.fn()}
        onDismiss={vi.fn()}
        onExpandReply={vi.fn()}
        onCollapseReply={vi.fn()}
        onReply={vi.fn()}
        onStop={vi.fn()}
      />,
    )

    const title = screen.getByText("First help m…straint")
    expect(title).toHaveClass("inline-block", "max-w-[52%]", "truncate")
    expect(document.querySelector("[data-pet-title-separator]")).toHaveClass(
      "h-1.5",
      "w-1.5",
      "rounded-full",
      "bg-popover-foreground/85",
    )
    expect(
      screen.getByText("The repository overview is ready for review.").parentElement,
    ).toHaveClass("line-clamp-2")
  })

  test("uses one stable middle ellipsis for a long CJK title", () => {
    render(
      <PetBubble
        activity={{
          ...runningActivity,
          title: "先帮我快速熟悉项目，说说开发时最该注意的地方。",
        }}
        expanded={false}
        interactionPending={false}
        onOpen={vi.fn()}
        onDismiss={vi.fn()}
        onExpandReply={vi.fn()}
        onCollapseReply={vi.fn()}
        onReply={vi.fn()}
        onStop={vi.fn()}
      />,
    )

    expect(screen.getByText("先帮我快速熟…地方。")).toBeInTheDocument()
  })

  test("shows the live assistant preview and exposes a hover dismiss action", () => {
    const onDismiss = vi.fn()
    render(
      <PetBubble
        activity={runningActivity}
        expanded={false}
        interactionPending={false}
        livePreview="Updating the interaction in real time"
        onOpen={vi.fn()}
        onDismiss={onDismiss}
        onExpandReply={vi.fn()}
        onCollapseReply={vi.fn()}
        onReply={vi.fn()}
        onStop={vi.fn()}
      />,
    )

    expect(screen.getByText("Updating the interaction in real time")).toBeInTheDocument()
    const dismiss = screen.getByRole("button", { name: "Dismiss" })
    expect(dismiss).toHaveClass("opacity-0", "group-hover:opacity-100")
    fireEvent.click(dismiss)
    expect(onDismiss).toHaveBeenCalledOnce()
  })

  test("uses the glass treatment, shimmers live output, and exposes stop on running hover", async () => {
    const onStop = vi.fn(async () => undefined)
    render(
      <PetBubble
        activity={runningActivity}
        expanded={false}
        interactionPending={false}
        livePreview="Planning the compact bubble"
        onOpen={vi.fn()}
        onDismiss={vi.fn()}
        onExpandReply={vi.fn()}
        onCollapseReply={vi.fn()}
        onReply={vi.fn()}
        onStop={onStop}
      />,
    )

    expect(screen.getByLabelText("Design the pet interaction")).toHaveClass(
      "bg-popover/75",
      "backdrop-blur-xl",
    )
    expect(screen.getByText("Planning the compact bubble")).toHaveClass("animate-text-shimmer")
    const stopButton = screen.getByRole("button", { name: "Stop reply" })
    expect(stopButton).toHaveClass("hover:bg-muted")
    expect(stopButton.querySelector("svg")).toHaveClass("fill-current")
    fireEvent.click(stopButton)
    await waitFor(() => expect(onStop).toHaveBeenCalledOnce())
  })

  test("exposes the same actions through the inactive-window hover bridge", () => {
    render(
      <PetBubble
        activity={runningActivity}
        expanded={false}
        interactionPending={false}
        onOpen={vi.fn()}
        onDismiss={vi.fn()}
        onExpandReply={vi.fn()}
        onCollapseReply={vi.fn()}
        onReply={vi.fn()}
        onStop={vi.fn()}
        nativeHovered
        nativeHoveredAction="stop"
      />,
    )

    expect(screen.getByRole("button", { name: "Dismiss" })).toHaveClass(
      "pointer-events-auto",
      "opacity-100",
    )
    const stopButton = screen.getByRole("button", { name: "Stop reply" })
    expect(stopButton.parentElement).toHaveClass("pointer-events-auto", "opacity-100")
    expect(stopButton).toHaveClass("bg-muted")
  })

  test("does not offer a free-form reply while an authoritative interaction is pending", () => {
    render(
      <PetBubble
        activity={{ ...runningActivity, status: "needs_input" }}
        expanded={false}
        interactionPending
        onOpen={vi.fn()}
        onDismiss={vi.fn()}
        onExpandReply={vi.fn()}
        onCollapseReply={vi.fn()}
        onReply={vi.fn()}
        onStop={vi.fn()}
      />,
    )
    expect(screen.queryByRole("button", { name: "Reply" })).not.toBeInTheDocument()
  })
})
