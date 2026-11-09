// @vitest-environment jsdom
import { useState } from "react"
import { afterEach, beforeEach, expect, test, vi } from "vitest"
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react"
import type { Transport } from "@/lib/transport"
import { WeComConnectionFields } from "./WeComFields"

const mocks = vi.hoisted(() => ({
  source: { call: vi.fn() },
  other: { call: vi.fn() },
  remote: false,
}))
vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => (mocks.remote ? mocks.other : mocks.source),
  useTransport: () => (mocks.remote ? mocks.other : mocks.source),
}))
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }))
function Harness() {
  const [botId, setBotId] = useState("bot-fixture")
  const [secret, setSecret] = useState("synthetic-fixture-secret")
  return (
    <WeComConnectionFields
      botId={botId}
      secret={secret}
      onBotIdChange={setBotId}
      onSecretChange={setSecret}
      sourceTransport={mocks.source as unknown as Transport}
    />
  )
}
beforeEach(() => {
  mocks.remote = false
  mocks.source.call.mockReset()
  mocks.other.call.mockReset()
})
afterEach(cleanup)

test("validation is explicit and only sends the selected WeCom credentials", async () => {
  mocks.source.call.mockResolvedValue("WeCom")
  render(<Harness />)
  expect(mocks.source.call).not.toHaveBeenCalled()
  fireEvent.click(screen.getByRole("button", { name: "channels.testConnection" }))
  await screen.findByText("channels.wecomValidated")
  expect(mocks.source.call).toHaveBeenCalledWith("channel_validate_credentials", {
    channelId: "wecom",
    credentials: { botId: "bot-fixture", secret: "synthetic-fixture-secret" },
  })
})
test("editing a secret invalidates old validation and errors never reflect provider text", async () => {
  let finish!: (name: string) => void
  mocks.source.call.mockReturnValueOnce(
    new Promise((resolve) => {
      finish = resolve
    }),
  )
  render(<Harness />)
  fireEvent.click(screen.getByRole("button", { name: "channels.testConnection" }))
  fireEvent.change(screen.getByLabelText("channels.wecomSecret"), {
    target: { value: "changed-fixture" },
  })
  await act(async () => finish("WeCom"))
  expect(screen.queryByText("channels.wecomValidated")).toBeNull()
  mocks.source.call.mockRejectedValue(new Error("synthetic-private-echo"))
  fireEvent.click(screen.getByRole("button", { name: "channels.testConnection" }))
  await screen.findByText("channels.wecomValidationFailed")
  expect(screen.queryByText(/synthetic-private-echo/)).toBeNull()
})
test("a host switch cannot send existing form credentials to the new runtime", async () => {
  let finish!: (name: string) => void
  mocks.source.call.mockReturnValue(
    new Promise((resolve) => {
      finish = resolve
    }),
  )
  const view = render(<Harness />)
  fireEvent.click(screen.getByRole("button", { name: "channels.testConnection" }))
  mocks.remote = true
  view.rerender(<Harness />)
  expect(screen.getByRole("button", { name: "channels.testConnection" })).toBeDisabled()
  await act(async () => finish("WeCom"))
  expect(screen.queryByText("channels.wecomValidated")).toBeNull()
  expect(mocks.other.call).not.toHaveBeenCalled()
})
