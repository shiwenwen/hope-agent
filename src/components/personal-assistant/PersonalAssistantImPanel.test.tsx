// @vitest-environment jsdom
import {
  act,
  cleanup,
  fireEvent,
  render,
  renderHook,
  screen,
  waitFor,
} from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import en from "@/i18n/locales/en.json"
import { TRANSPORT_EVENT_RESYNC_REQUIRED } from "@/lib/transport"
import type { SessionMeta } from "@/types/chat"
import { PersonalAssistantImPanel } from "./PersonalAssistantImPanel"
import { PersonalAssistantPanel } from "./PersonalAssistantPanel"
import { PersonalAssistantSettings } from "./PersonalAssistantSettings"
import type { PersonalAssistantState } from "./usePersonalAssistant"
import { usePersonalAssistantIm, type AssistantImSnapshot } from "./usePersonalAssistantIm"

const fixture = vi.hoisted(() => ({
  revision: 1,
  call: vi.fn(),
  listeners: new Map<string, () => void>(),
  listen: vi.fn((event: string, callback: () => void) => {
    fixture.listeners.set(event, callback)
    return () => fixture.listeners.delete(event)
  }),
}))
vi.mock("@/lib/transport-provider", () => ({
  useTransport: () => fixture,
  useTransportRevision: () => fixture.revision,
  getTransportRevision: () => fixture.revision,
}))
vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: string | Record<string, unknown>) => {
      const value = key
        .split(".")
        .reduce<unknown>(
          (value, segment) =>
            value && typeof value === "object"
              ? (value as Record<string, unknown>)[segment]
              : undefined,
          en,
        )
      const text = String(value ?? (typeof options === "string" ? options : key))
      return typeof options === "object"
        ? text.replace(/{{(\w+)}}/g, (_, name: string) => String(options[name] ?? name))
        : text
    },
    i18n: { language: "en" },
  }),
}))
vi.mock("@/lib/openExternalUrl", () => ({ openExternalUrl: vi.fn() }))
const work = { id: "work-a", title: "Release work", kind: "regular" } as SessionMeta
const connection = {
  connectionId: "dedicated-connection",
  botId: 11,
  botUsername: "test_assistant_bot",
  generation: 1,
  state: "unpaired" as const,
  notificationsPaused: false,
}
let snapshot: AssistantImSnapshot
let operation: (command: string, args?: Record<string, unknown>) => unknown

beforeEach(() => {
  fixture.revision = 1
  fixture.listeners.clear()
  snapshot = {
    connection: { ...connection },
    grants: [],
    deliveries: [],
    notificationPolicy: {
      timezone: "Asia/Shanghai",
      quietStartHour: 22,
      quietEndHour: 8,
      dailyLimit: 2,
    },
  }
  operation = () => undefined
  fixture.call
    .mockReset()
    .mockImplementation(async (command: string, args?: Record<string, unknown>) =>
      command === "personal_assistant_im_status"
        ? structuredClone(snapshot)
        : operation(command, args),
    )
  Object.defineProperty(document, "visibilityState", { configurable: true, value: "visible" })
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  )
  HTMLElement.prototype.scrollIntoView = vi.fn()
  HTMLElement.prototype.hasPointerCapture = vi.fn().mockReturnValue(false)
})
afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})
const changed = () => act(() => fixture.listeners.get("personal_assistant_im:changed")?.())
const button = (name: string) => screen.getByRole("button", { name })

// Use real panel, public form controls, and owner-state hook. The transport is
// synthetic: these tests neither contact Telegram nor create real credentials.
describe("assistant IM owner settings", () => {
  it("waits for phone activation after exact identity confirmation and drops grants on unpair", async () => {
    operation = (command) => {
      if (command === "personal_assistant_im_begin_pairing") {
        snapshot.connection = { ...connection, generation: 2, state: "awaiting_code" }
        return {
          connectionId: connection.connectionId,
          generation: 2,
          code: "test-only-code",
          expiresAt: Date.now() / 1000 + 300,
        }
      }
      if (command === "personal_assistant_im_confirm_pairing")
        snapshot.connection = { ...snapshot.connection!, state: "awaiting_activation" }
      if (command === "personal_assistant_im_revoke_pairing") {
        snapshot.connection = { ...connection, generation: 3 }
        snapshot.grants = []
      }
    }
    render(<PersonalAssistantImPanel sessions={[work]} />)
    fireEvent.click(await screen.findByRole("button", { name: en.personalAssistant.im.pair }))
    await screen.findByText("/ha_pair test-only-code")
    expect(screen.queryByLabelText(en.personalAssistant.im.work)).toBeNull()
    snapshot.connection = {
      ...snapshot.connection!,
      state: "awaiting_confirmation",
      userId: 23,
      chatId: 23,
    }
    changed()
    fireEvent.click(await screen.findByRole("button", { name: en.personalAssistant.im.confirm }))
    await screen.findByText(en.personalAssistant.im.state.awaiting_activation)
    expect(fixture.call).toHaveBeenCalledWith("personal_assistant_im_confirm_pairing", {
      generation: 2,
      userId: 23,
      chatId: 23,
    })
    expect(screen.queryByLabelText(en.personalAssistant.im.work)).toBeNull()
    expect(screen.queryByText("/ha_pair test-only-code")).toBeNull()
    snapshot.connection = { ...snapshot.connection!, state: "active" }
    snapshot.grants = [
      {
        sessionId: work.id,
        revision: 1,
        queryStatus: true,
        delegate: true,
        stop: false,
        completionNotice: false,
        expiresAt: Date.now() / 1000 + 86400,
      },
    ]
    changed()
    await screen.findByLabelText(en.personalAssistant.im.work)
    await waitFor(() =>
      expect((button(en.personalAssistant.im.unpair) as HTMLButtonElement).disabled).toBe(false),
    )
    fireEvent.click(button(en.personalAssistant.im.unpair))
    await screen.findByText(en.personalAssistant.im.state.unpaired)
    expect(screen.queryByLabelText(en.personalAssistant.im.work)).toBeNull()
    expect(screen.queryByText("Release work")).toBeNull()
    expect(
      fixture.call.mock.calls.every(([command]) => command.startsWith("personal_assistant_im_")),
    ).toBe(true)
  })

  it("hides a generated code after another client starts a new pairing generation", async () => {
    operation = () => {
      snapshot.connection = { ...connection, state: "awaiting_code" }
      return {
        connectionId: connection.connectionId,
        generation: 1,
        code: "old-code",
        expiresAt: Date.now() / 1000 + 300,
      }
    }
    render(<PersonalAssistantImPanel sessions={[]} />)
    fireEvent.click(await screen.findByRole("button", { name: en.personalAssistant.im.pair }))
    await screen.findByText("/ha_pair old-code")
    snapshot.connection = { ...connection, state: "awaiting_code", generation: 2 }
    changed()
    await waitFor(() => expect(screen.queryByText("/ha_pair old-code")).toBeNull())
  })

  it("shows a work-bot conflict without claiming a connection or retaining the typed token", async () => {
    snapshot.connection = null
    operation = () => {
      throw new Error("assistant_im_identity_in_use")
    }
    render(<PersonalAssistantImPanel sessions={[]} />)
    const input = await screen.findByLabelText(en.personalAssistant.im.token)
    fireEvent.change(input, { target: { value: "synthetic-bot-token" } })
    fireEvent.click(button(en.personalAssistant.im.connect))
    await screen.findByText(en.personalAssistant.im.identityInUse)
    expect((input as HTMLInputElement).value).toBe("")
    expect(screen.queryByRole("button", { name: en.personalAssistant.im.pair })).toBeNull()
    expect(fixture.call).toHaveBeenCalledWith("personal_assistant_im_configure", {
      token: "synthetic-bot-token",
    })
  })

  it("starts permissions disabled and removes dependent permissions when status access is disabled", async () => {
    snapshot.connection = { ...connection, state: "active" }
    render(<PersonalAssistantImPanel sessions={[work]} />)
    const choose = await screen.findByRole("combobox")
    fireEvent.keyDown(choose, { key: "ArrowDown" })
    fireEvent.click(await screen.findByRole("option", { name: work.title || work.id }))
    const status = screen.getByRole("switch", { name: en.personalAssistant.im.queryStatus })
    const delegate = screen.getByRole("switch", { name: en.personalAssistant.im.delegate })
    expect(status.getAttribute("aria-checked")).toBe("false")
    expect((delegate as HTMLButtonElement).disabled).toBe(true)
    expect((button(en.common.save) as HTMLButtonElement).disabled).toBe(true)
    fireEvent.click(status)
    fireEvent.click(delegate)
    fireEvent.click(status)
    expect(delegate.getAttribute("aria-checked")).toBe("false")
    fireEvent.click(status)
    fireEvent.click(button(en.common.save))
    await waitFor(() =>
      expect(fixture.call).toHaveBeenCalledWith("personal_assistant_im_set_grant", {
        input: expect.objectContaining({
          sessionId: work.id,
          revision: 0,
          queryStatus: true,
          delegate: false,
          stop: false,
          completionNotice: false,
        }),
      }),
    )
  })

  it("renews expired access with its current revision and never retries an unknown receipt", async () => {
    snapshot.connection = { ...connection, state: "active" }
    snapshot.grants = [
      {
        sessionId: work.id,
        revision: 7,
        queryStatus: true,
        delegate: false,
        stop: false,
        completionNotice: false,
        expiresAt: 1,
      },
    ]
    snapshot.deliveries = [
      { id: "receipt", requestId: "request", state: "unknown", createdAt: 1, deferredReason: null },
    ]
    render(<PersonalAssistantImPanel sessions={[work]} />)
    await screen.findByText(en.personalAssistant.im.delivery.unknown)
    expect(screen.getByText("Owner time zone: Asia/Shanghai")).toBeTruthy()
    expect(screen.getByText(/quiet from 22:00 to 08:00; at most 2 per local day/)).toBeTruthy()
    expect(screen.queryByText(en.personalAssistant.im.notificationTimezoneRequired)).toBeNull()
    fireEvent.click(button(en.common.edit))
    const expiry = screen.getByLabelText(en.personalAssistant.im.expires)
    expect((button(en.common.save) as HTMLButtonElement).disabled).toBe(true)
    const future = new Date(Date.now() + 86400000)
    fireEvent.change(expiry, {
      target: {
        value: `${future.getFullYear()}-${String(future.getMonth() + 1).padStart(2, "0")}-${String(future.getDate()).padStart(2, "0")}`,
      },
    })
    fireEvent.click(button(en.common.save))
    await waitFor(() =>
      expect(fixture.call).toHaveBeenCalledWith("personal_assistant_im_set_grant", {
        input: expect.objectContaining({ revision: 7 }),
      }),
    )
    expect(
      fixture.call.mock.calls
        .map(([command]) => command)
        .filter((command) => command !== "personal_assistant_im_status"),
    ).toEqual(["personal_assistant_im_set_grant"])
    snapshot.notificationPolicy.timezone = null
    snapshot.deliveries.push({
      id: "deferred",
      requestId: "completion",
      state: "pending",
      createdAt: 2,
      deferredReason: "timezone_required",
    })
    changed()
    await screen.findByText(en.personalAssistant.im.notificationTimezoneRequired)
    await screen.findByText(en.personalAssistant.im.deferred.timezone_required)
    expect(screen.queryByText("Owner time zone: Asia/Shanghai")).toBeNull()
    snapshot.notificationPolicy.timezone = "Asia/Shanghai"
    for (const reason of ["quiet_hours", "daily_limit"] as const) {
      snapshot.deliveries[1].deferredReason = reason
      changed()
      await screen.findByText(en.personalAssistant.im.deferred[reason])
    }
    snapshot.deliveries[1].state = "accepted"
    snapshot.deliveries[1].deferredReason = null
    changed()
    await screen.findByText(en.personalAssistant.im.delivery.accepted)
    expect(screen.queryByText(en.personalAssistant.im.deferred.daily_limit)).toBeNull()
  })
})

describe("assistant IM private scope", () => {
  it("clears typed tokens and one-time codes when the backend changes or the panel closes", async () => {
    snapshot.connection = null
    const { rerender } = render(<PersonalAssistantImPanel sessions={[]} />)
    fireEvent.change(await screen.findByLabelText(en.personalAssistant.im.token), {
      target: { value: "unsubmitted-secret" },
    })
    fixture.revision++
    rerender(<PersonalAssistantImPanel sessions={[]} />)
    expect(
      ((await screen.findByLabelText(en.personalAssistant.im.token)) as HTMLInputElement).value,
    ).toBe("")
    fireEvent.change(screen.getByLabelText(en.personalAssistant.im.token), {
      target: { value: "second-secret" },
    })
    rerender(<PersonalAssistantImPanel sessions={[]} visible={false} />)
    rerender(<PersonalAssistantImPanel sessions={[]} />)
    expect(
      ((await screen.findByLabelText(en.personalAssistant.im.token)) as HTMLInputElement).value,
    ).toBe("")
    snapshot.connection = { ...connection }
    operation = () => {
      snapshot.connection = { ...connection, state: "awaiting_code" }
      return {
        connectionId: connection.connectionId,
        generation: 1,
        code: "one-time-secret",
        expiresAt: Date.now() / 1000 + 300,
      }
    }
    changed()
    fireEvent.click(await screen.findByRole("button", { name: en.personalAssistant.im.pair }))
    await screen.findByText("/ha_pair one-time-secret")
    fixture.revision++
    rerender(<PersonalAssistantImPanel sessions={[]} />)
    await screen.findByRole("button", { name: en.personalAssistant.im.restartPairing })
    expect(screen.queryByText("/ha_pair one-time-secret")).toBeNull()
    fireEvent.click(button(en.personalAssistant.im.restartPairing))
    await screen.findByText("/ha_pair one-time-secret")
    rerender(<PersonalAssistantImPanel sessions={[]} visible={false} />)
    rerender(<PersonalAssistantImPanel sessions={[]} />)
    await screen.findByRole("button", { name: en.personalAssistant.im.restartPairing })
    expect(screen.queryByText("/ha_pair one-time-secret")).toBeNull()
  })

  it("keeps IM configuration in assistant settings and reads it only while the visible IM tab is open", async () => {
    const state = {
      authorized: true,
      loading: false,
      error: null,
      requests: [],
      watches: [],
      refresh: vi.fn(),
      cancel: vi.fn(),
      watchGoal: vi.fn(),
      open: vi.fn(),
      profile: { name: "Hope", avatar: null },
      saveProfile: vi.fn(),
    } as PersonalAssistantState
    const panel = render(
      <PersonalAssistantPanel state={state} onOpenRequest={vi.fn()} onOpenGoal={vi.fn()} />,
    )
    expect(screen.queryByRole("tab", { name: en.personalAssistant.im.title })).toBeNull()
    panel.unmount()
    const { rerender } = render(
      <PersonalAssistantSettings open onOpenChange={vi.fn()} state={state} sessions={[work]} />,
    )
    expect(fixture.call).not.toHaveBeenCalled()
    fireEvent.mouseDown(screen.getByRole("tab", { name: en.personalAssistant.settings.im }), {
      button: 0,
      ctrlKey: false,
    })
    await screen.findByRole("button", { name: en.personalAssistant.im.pair })
    fireEvent.mouseDown(screen.getByRole("tab", { name: en.personalAssistant.settings.profile }), {
      button: 0,
      ctrlKey: false,
    })
    await waitFor(() => expect(fixture.listeners.has("personal_assistant_im:changed")).toBe(false))
    const attempts = fixture.call.mock.calls.length
    changed()
    expect(fixture.call).toHaveBeenCalledTimes(attempts)
    fireEvent.mouseDown(screen.getByRole("tab", { name: en.personalAssistant.settings.im }), {
      button: 0,
      ctrlKey: false,
    })
    await screen.findByRole("button", { name: en.personalAssistant.im.pair })
    rerender(
      <PersonalAssistantSettings
        open
        onOpenChange={vi.fn()}
        visible={false}
        state={state}
        sessions={[work]}
      />,
    )
    expect(screen.queryByRole("button", { name: en.personalAssistant.im.pair })).toBeNull()
    expect(fixture.listeners.has("personal_assistant_im:changed")).toBe(false)
  })

  it("clears a failed private read, suppresses passive retry, and verifies after reconnect", async () => {
    const { result } = renderHook(usePersonalAssistantIm)
    await waitFor(() => expect(result.current.data?.connection?.botId).toBe(11))
    fixture.call.mockRejectedValue(new Error("Owner access revoked"))
    changed()
    await waitFor(() => expect(result.current.data).toBeNull())
    expect(result.current.error).toContain("Owner access revoked")
    const attempts = fixture.call.mock.calls.length
    changed()
    expect(fixture.call).toHaveBeenCalledTimes(attempts)
    fixture.call.mockResolvedValue(snapshot)
    act(() => fixture.listeners.get(TRANSPORT_EVENT_RESYNC_REQUIRED)?.())
    await waitFor(() => expect(result.current.data?.connection?.botId).toBe(11))
  })

  it("discards a delayed mutation from a previous owner scope and prevents duplicate writes", async () => {
    let resolveOld!: (value: unknown) => void
    operation = () =>
      new Promise((resolve) => {
        resolveOld = resolve
      })
    const { result, rerender } = renderHook(usePersonalAssistantIm)
    await waitFor(() => expect(result.current.data).not.toBeNull())
    let old!: Promise<unknown>
    act(() => {
      old = result.current.mutate("begin_pairing")
    })
    await act(async () => {
      expect(await result.current.mutate("begin_pairing")).toBeUndefined()
    })
    expect(
      fixture.call.mock.calls.filter(
        ([command]) => command === "personal_assistant_im_begin_pairing",
      ),
    ).toHaveLength(1)
    snapshot = { ...snapshot, connection: null, grants: [], deliveries: [] }
    fixture.revision++
    rerender()
    await waitFor(() => expect(result.current.data?.connection).toBeNull())
    await act(async () => {
      resolveOld({ code: "old-secret" })
      expect(await old).toBeUndefined()
    })
    expect(result.current.data?.connection).toBeNull()
    expect(result.current.feedback).toBeNull()
  })
})
