// @vitest-environment jsdom

import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, test, vi } from "vitest"
import { TooltipProvider } from "@/components/ui/tooltip"
import type { MessageAttachment } from "@/types/chat"
import UserAttachments from "./UserAttachments"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { defaultValue?: string }) => options?.defaultValue ?? key,
  }),
}))

vi.mock("sonner", () => ({
  toast: {
    error: vi.fn(),
    success: vi.fn(),
  },
}))

vi.mock("@/components/common/ImageLightbox", () => ({
  useLightbox: () => ({ openLightbox: vi.fn() }),
}))

vi.mock("@/lib/logger", () => ({
  logger: {
    error: vi.fn(),
  },
}))

const transportMock = vi.hoisted(() => ({
  supportsLocalFileOps: vi.fn(() => true),
  resolveMediaUrl: vi.fn(() => null),
  openMedia: vi.fn(),
  downloadMedia: vi.fn(),
  revealMedia: vi.fn(),
  call: vi.fn(),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
}))

afterEach(() => {
  cleanup()
  vi.useRealTimers()
  vi.restoreAllMocks()
  vi.clearAllMocks()
})

function renderUserAttachments(attachments: MessageAttachment[]) {
  return render(
    <TooltipProvider>
      <UserAttachments attachments={attachments} />
    </TooltipProvider>,
  )
}

describe("UserAttachments", () => {
  test("keeps optimistic blob preview alive across immediate remount", async () => {
    vi.useFakeTimers()
    const revokeObjectURL = vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => {})
    const attachment: MessageAttachment = {
      name: "photo.png",
      mimeType: "image/png",
      sizeBytes: 12,
      kind: "image",
      previewUrl: "blob:hope-agent-preview",
    }

    const first = renderUserAttachments([attachment])
    expect(screen.getByRole("img", { name: "photo.png" }).getAttribute("src")).toBe(
      "blob:hope-agent-preview",
    )

    first.unmount()
    const second = renderUserAttachments([attachment])
    await vi.runOnlyPendingTimersAsync()

    expect(screen.getByRole("img", { name: "photo.png" }).getAttribute("src")).toBe(
      "blob:hope-agent-preview",
    )
    expect(revokeObjectURL).not.toHaveBeenCalled()

    second.unmount()
    await vi.runOnlyPendingTimersAsync()

    expect(revokeObjectURL).toHaveBeenCalledWith("blob:hope-agent-preview")
  })
})
