// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, expect, test, vi } from "vitest"
import { AssistantUpdateEntry } from "./AssistantUpdateEntry"

vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }))
vi.mock("sonner", () => ({ toast: { error: vi.fn() } }))
afterEach(cleanup)

const update = {
  id: "update-1",
  itemId: "item-1",
  kind: "result",
  summary: "The result was checked.",
  resultMessageId: 12,
  createdAt: "2026-11-15T10:00:00Z",
  viewedAt: null,
}

test("showing a background update does not mark it read until an explicit action", async () => {
  const onView = vi.fn().mockResolvedValue(undefined)
  const { rerender } = render(<AssistantUpdateEntry update={update} name="Hope" onView={onView} />)
  expect(onView).not.toHaveBeenCalled()
  fireEvent.click(screen.getByRole("button"))
  await waitFor(() => expect(onView).toHaveBeenCalledExactlyOnceWith(update.id))
  rerender(
    <AssistantUpdateEntry
      update={{ ...update, viewedAt: update.createdAt }}
      name="Hope"
      onView={onView}
    />,
  )
  expect(screen.queryByRole("button")).toBeNull()
})

test("a failed read receipt remains retryable", async () => {
  const onView = vi
    .fn()
    .mockRejectedValueOnce(new Error("disconnected"))
    .mockResolvedValue(undefined)
  render(<AssistantUpdateEntry update={update} name="Hope" onView={onView} />)
  fireEvent.click(screen.getByRole("button"))
  await waitFor(() => expect(screen.getByRole("button")).toBeEnabled())
  fireEvent.click(screen.getByRole("button"))
  await waitFor(() => expect(onView).toHaveBeenCalledTimes(2))
})
