// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import "@testing-library/jest-dom/vitest"
import { afterEach, describe, expect, test, vi } from "vitest"
import type { ApprovalRequest } from "@/components/chat/ApprovalDialog"
import { PetApprovalCard } from "./PetApprovalCard"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { defaultValue?: string }) => options?.defaultValue ?? key,
  }),
}))

afterEach(cleanup)

const request: ApprovalRequest = {
  request_id: "approval-pet",
  session_id: "session-pet",
  command: "pnpm typecheck",
  cwd: "/workspace",
  reason: { kind: "edit_command", detail: "Writes generated output" },
}

describe("PetApprovalCard", () => {
  test("uses the compact Pet actions while preserving the approval protocol", async () => {
    const onRespond = vi.fn(async () => undefined)
    render(<PetApprovalCard request={request} onRespond={onRespond} />)

    expect(screen.getByText("pnpm typecheck")).toBeInTheDocument()
    fireEvent.click(screen.getByRole("button", { name: "Allow once" }))
    await waitFor(() => expect(onRespond).toHaveBeenCalledWith("approval-pet", "allow_once"))
  })

  test("does not offer a standing grant for strict approval reasons", () => {
    render(
      <PetApprovalCard
        request={{ ...request, reason: { kind: "dangerous_command" } }}
        onRespond={vi.fn()}
      />,
    )

    expect(screen.queryByRole("button", { name: "Always allow" })).not.toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Allow once" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Deny" })).toBeInTheDocument()
  })
})
