// @vitest-environment jsdom
import { afterAll, afterEach, beforeAll, beforeEach, expect, test, vi } from "vitest"
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import AddAccountDialog from "./AddAccountDialog"
import EditAccountDialog from "./EditAccountDialog"
import type { ChannelAccountConfig, ChannelPluginInfo } from "./types"

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
vi.mock("@/lib/logger", () => ({ logger: { error: vi.fn() } }))
vi.mock("./WeChatConnectSection", () => ({ default: () => null }))
vi.mock("./TelegramConnectionFields", () => ({ default: () => null }))
vi.mock("./TelegramGroupConfig", () => ({ default: () => null }))
vi.mock("@/components/common/ChannelIcon", () => ({ default: () => null }))
const scrollIntoView = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "scrollIntoView")
beforeAll(() => {
  Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
    configurable: true,
    value: vi.fn(),
  })
})
afterAll(() => {
  if (scrollIntoView) Object.defineProperty(HTMLElement.prototype, "scrollIntoView", scrollIntoView)
  else Reflect.deleteProperty(HTMLElement.prototype, "scrollIntoView")
})

const account: ChannelAccountConfig = {
  id: "wecom-fixture",
  channelId: "wecom",
  label: "Fixture",
  enabled: true,
  credentials: { botId: "bot-fixture", secret: "synthetic-secret" },
  settings: { imReplyMode: "final", customSetting: "preserve" },
  notifyStartup: false,
  notifySessionEviction: false,
  security: {
    dmPolicy: "allowlist",
    userAllowlist: ["alice"],
    groupAllowlist: [],
    adminIds: [],
    groupPolicy: "allowlist",
    groups: {
      "group:old": {
        requireMention: true,
        enabled: false,
        allowFrom: ["owner"],
        agentId: "private-agent",
        topics: {},
      },
    },
    channels: {},
  },
}
beforeEach(() => {
  mocks.remote = false
  mocks.source.call.mockReset().mockResolvedValue("wecom-fixture")
  mocks.other.call.mockReset()
})
afterEach(cleanup)

test("new WeCom accounts start with allowlists and only explicitly entered group IDs", async () => {
  const added = vi.fn()
  render(<AddAccountDialog open onOpenChange={vi.fn()} onAdded={added} initialChannelId="wecom" />)
  fireEvent.change(await screen.findByLabelText("channels.wecomBotId"), {
    target: { value: "bot-fixture" },
  })
  fireEvent.change(screen.getByLabelText("channels.wecomSecret"), {
    target: { value: "synthetic-secret" },
  })
  fireEvent.change(screen.getByPlaceholderText("channels.accountLabelPlaceholder"), {
    target: { value: "Work" },
  })
  fireEvent.change(screen.getByLabelText("channels.wecomGroups"), {
    target: { value: "room-one\nroom-two" },
  })
  fireEvent.click(screen.getByRole("button", { name: "common.save" }))
  await waitFor(() => expect(added).toHaveBeenCalledOnce())
  expect(mocks.source.call).toHaveBeenCalledWith(
    "channel_add_account",
    expect.objectContaining({
      channelId: "wecom",
      credentials: { botId: "bot-fixture", secret: "synthetic-secret" },
      settings: { imReplyMode: "final" },
      security: expect.objectContaining({
        dmPolicy: "allowlist",
        userAllowlist: [],
        groupPolicy: "allowlist",
        groups: {
          "group:room-one": { requireMention: false, enabled: true, allowFrom: [], topics: {} },
          "group:room-two": { requireMention: false, enabled: true, allowFrom: [], topics: {} },
        },
      }),
    }),
  )
})

test("editing the label preserves strict groups, credentials and unrelated account settings", async () => {
  const saved = vi.fn()
  render(
    <EditAccountDialog
      open
      onOpenChange={vi.fn()}
      onSaved={saved}
      account={account}
      plugins={[]}
      agents={[]}
    />,
  )
  fireEvent.change(screen.getByPlaceholderText("channels.accountLabelPlaceholder"), {
    target: { value: "Renamed" },
  })
  fireEvent.click(screen.getByRole("button", { name: "common.save" }))
  await waitFor(() => expect(saved).toHaveBeenCalledOnce())
  expect(mocks.source.call).toHaveBeenCalledWith(
    "channel_update_account",
    expect.objectContaining({
      accountId: account.id,
      label: "Renamed",
      security: expect.objectContaining({ groups: account.security.groups }),
      settings: expect.objectContaining({ imReplyMode: "final", customSetting: "preserve" }),
    }),
  )
  const params = mocks.source.call.mock.calls.find(
    ([name]) => name === "channel_update_account",
  )?.[1]
  expect(params).not.toHaveProperty("credentials")
})

test("an open WeCom editor cannot write a stale account to a different runtime", async () => {
  const saved = vi.fn()
  const props = {
    open: true,
    onOpenChange: vi.fn(),
    onSaved: saved,
    account,
    plugins: [],
    agents: [],
  }
  const view = render(<EditAccountDialog {...props} />)
  fireEvent.change(screen.getByPlaceholderText("channels.accountLabelPlaceholder"), {
    target: { value: "Renamed" },
  })
  mocks.remote = true
  view.rerender(<EditAccountDialog {...props} />)
  fireEvent.click(screen.getByRole("button", { name: "common.save" }))
  expect(mocks.other.call).not.toHaveBeenCalled()
  expect(mocks.source.call).not.toHaveBeenCalled()
  expect(saved).not.toHaveBeenCalled()
  expect(screen.getAllByText("channels.wecomHostChanged").length).toBeGreaterThan(0)
})

test("a completed old-host save cannot report the account as saved in a new runtime", async () => {
  let finish!: (value: unknown) => void
  mocks.source.call.mockReturnValue(
    new Promise((resolve) => {
      finish = resolve
    }),
  )
  const saved = vi.fn()
  const props = {
    open: true,
    onOpenChange: vi.fn(),
    onSaved: saved,
    account,
    plugins: [],
    agents: [],
  }
  const view = render(<EditAccountDialog {...props} />)
  fireEvent.change(screen.getByPlaceholderText("channels.accountLabelPlaceholder"), {
    target: { value: "Renamed" },
  })
  fireEvent.click(screen.getByRole("button", { name: "common.save" }))
  mocks.remote = true
  view.rerender(<EditAccountDialog {...props} />)
  await act(async () => finish(null))
  expect(saved).not.toHaveBeenCalled()
  expect(mocks.other.call).not.toHaveBeenCalled()
  expect(screen.getAllByText("channels.wecomHostChanged").length).toBeGreaterThan(0)
})

test("closing and reopening new-account setup does not carry secrets into another runtime", async () => {
  const props = { onOpenChange: vi.fn(), onAdded: vi.fn(), initialChannelId: "wecom" }
  const view = render(<AddAccountDialog {...props} open />)
  fireEvent.change(await screen.findByLabelText("channels.wecomBotId"), {
    target: { value: "old-host-bot" },
  })
  fireEvent.change(screen.getByLabelText("channels.wecomSecret"), {
    target: { value: "synthetic-secret" },
  })
  view.rerender(<AddAccountDialog {...props} open={false} />)
  mocks.remote = true
  view.rerender(<AddAccountDialog {...props} open />)
  expect(await screen.findByLabelText("channels.wecomBotId")).toHaveValue("")
  expect(screen.getByLabelText("channels.wecomSecret")).toHaveValue("")
  expect(mocks.other.call).not.toHaveBeenCalled()
})

const imessagePlugin: ChannelPluginInfo = {
  meta: { id: "imessage", displayName: "iMessage", description: "fixture", version: "test" },
  capabilities: {
    chatTypes: ["dm"],
    supportsPolls: false,
    supportsReactions: false,
    supportsEdit: false,
    supportsMedia: [],
    supportsTyping: false,
    streamingPreviewMaxBytes: null,
  },
}

test.each(["back", "reopen"])(
  "leaving WeCom through %s restores another channel's original new-account defaults",
  async (navigation) => {
    const added = vi.fn()
    const props = { onOpenChange: vi.fn(), onAdded: added, plugins: [imessagePlugin] }
    const view = render(<AddAccountDialog {...props} open initialChannelId="wecom" />)
    fireEvent.change(await screen.findByLabelText("channels.wecomGroups"), {
      target: { value: "wecom-only-room" },
    })
    if (navigation === "back") {
      fireEvent.click(screen.getByRole("button", { name: "common.back" }))
      fireEvent.click(screen.getByRole("button", { name: /channels.pluginName_imessage/ }))
    } else {
      view.rerender(<AddAccountDialog {...props} open={false} initialChannelId="wecom" />)
      view.rerender(<AddAccountDialog {...props} open initialChannelId="imessage" />)
    }
    fireEvent.change(await screen.findByPlaceholderText("channels.accountLabelPlaceholder"), {
      target: { value: "Other channel" },
    })
    fireEvent.click(screen.getByRole("button", { name: "common.save" }))
    await waitFor(() => expect(added).toHaveBeenCalledOnce())
    expect(mocks.source.call).toHaveBeenCalledExactlyOnceWith(
      "channel_add_account",
      expect.objectContaining({
        channelId: "imessage",
        credentials: {},
        security: expect.objectContaining({
          dmPolicy: "open",
          groupPolicy: "open",
          userAllowlist: [],
          groups: {},
          channels: {},
        }),
      }),
    )
  },
)

test("a reopened WeCom draft starts denied even after an earlier draft selected open DMs", async () => {
  const props = { onOpenChange: vi.fn(), onAdded: vi.fn(), initialChannelId: "wecom" }
  const view = render(<AddAccountDialog {...props} open />)
  const section = (await screen.findByText("channels.dmPolicy")).parentElement!
  fireEvent.click(within(section).getByRole("combobox"))
  fireEvent.click(await screen.findByRole("option", { name: "channels.dmPolicyOpen" }))
  view.rerender(<AddAccountDialog {...props} open={false} />)
  view.rerender(<AddAccountDialog {...props} open />)
  fireEvent.change(await screen.findByLabelText("channels.wecomBotId"), {
    target: { value: "bot-fixture" },
  })
  fireEvent.change(screen.getByLabelText("channels.wecomSecret"), {
    target: { value: "synthetic-secret" },
  })
  fireEvent.change(screen.getByPlaceholderText("channels.accountLabelPlaceholder"), {
    target: { value: "New draft" },
  })
  fireEvent.click(screen.getByRole("button", { name: "common.save" }))
  await waitFor(() => expect(props.onAdded).toHaveBeenCalledOnce())
  expect(mocks.source.call).toHaveBeenCalledExactlyOnceWith(
    "channel_add_account",
    expect.objectContaining({
      channelId: "wecom",
      security: expect.objectContaining({
        dmPolicy: "allowlist",
        groupPolicy: "allowlist",
        userAllowlist: [],
        groups: {},
      }),
    }),
  )
})
