// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import en from "@/i18n/locales/en.json"
import { PersonalAssistantSettings } from "./PersonalAssistantSettings"
import { PersonalAssistantHeader } from "./PersonalAssistantHeader"
import { PersonalAssistantWelcome } from "./PersonalAssistantWelcome"
import { usePersonalAssistant, type AssistantProfile } from "./usePersonalAssistant"

const fixture = vi.hoisted(() => ({
  revision: 1,
  call: vi.fn(),
  listen: vi.fn(() => () => {}),
  resolveAssetUrl: vi.fn((path: string) => `/api/avatars/${path.split("/").pop()}`),
  pickLocalImage: vi.fn(async () => ({ src: "blob:local-picture", revoke: vi.fn() })),
  prepareFileData: vi.fn(() => [1, 2, 3]),
  readBuffer: vi.fn(async () => new ArrayBuffer(3)),
}))
vi.mock("@/lib/transport-provider", () => ({
  useTransport: () => fixture,
  getTransport: () => fixture,
  useTransportRevision: () => fixture.revision,
  getTransportRevision: () => fixture.revision,
}))
vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, args?: Record<string, unknown>) => {
      const value = key
        .split(".")
        .reduce<unknown>(
          (node, segment) =>
            node && typeof node === "object"
              ? (node as Record<string, unknown>)[segment]
              : undefined,
          en,
        )
      return String(value ?? key).replace(/{{(\w+)}}/g, (_, name: string) =>
        String(args?.[name] ?? name),
      )
    },
  }),
}))
vi.mock("@/components/settings/AvatarCropDialog", () => ({
  AvatarCropDialog: ({
    onConfirm,
    onCancel,
  }: {
    onConfirm: (blob: Blob) => void
    onCancel: () => void
  }) => (
    <div>
      <button
        type="button"
        onClick={() =>
          onConfirm(
            Object.assign(new Blob([], { type: "image/png" }), { arrayBuffer: fixture.readBuffer }),
          )
        }
      >
        Confirm crop
      </button>
      <button type="button" onClick={onCancel}>
        Cancel crop
      </button>
    </div>
  ),
}))

let profile: AssistantProfile
let write: ((input: AssistantProfile) => Promise<AssistantProfile>) | undefined
beforeEach(() => {
  fixture.revision = 1
  fixture.readBuffer.mockReset().mockResolvedValue(new ArrayBuffer(3))
  profile = { name: "Hope", avatar: null }
  write = undefined
  fixture.call
    .mockReset()
    .mockImplementation(async (command: string, args?: { profile?: AssistantProfile }) => {
      if (command === "get_personal_assistant_profile") return { ...profile }
      if (command === "update_personal_assistant_profile") {
        const next = args!.profile!
        if (write) return write(next)
        profile = { ...next }
        return { ...profile }
      }
      if (command === "save_avatar") return "/isolated/avatars/avatar.png"
      return []
    })
})
afterEach(cleanup)

function Harness() {
  const state = usePersonalAssistant()
  return (
    <>
      <PersonalAssistantHeader
        profile={state.profile}
        attentionCount={0}
        itemsOpen={false}
        onOpenSettings={vi.fn()}
        onToggleItems={vi.fn()}
        onSearch={vi.fn()}
        workbench={{
          width: 0,
          layoutMode: "docked",
          tabs: [],
          launchItems: [],
          activeId: null,
          docked: false,
          onExpand: vi.fn(),
          onSelect: vi.fn(),
          onOpen: vi.fn(),
          onCloseTab: vi.fn(),
          onReorder: vi.fn(),
          onCloseWorkbench: vi.fn(),
        }}
      />
      <PersonalAssistantWelcome profile={state.profile} />
      <PersonalAssistantSettings state={state} sessions={[]} open onOpenChange={vi.fn()} />
    </>
  )
}

describe("personal assistant profile settings", () => {
  it("does not upload an old owner's image after switching connections while reading the crop", async () => {
    let resolve!: (value: ArrayBuffer) => void
    fixture.readBuffer.mockImplementation(
      () =>
        new Promise((done) => {
          resolve = done
        }),
    )
    const { rerender } = render(<Harness />)
    await screen.findByRole("textbox", { name: en.personalAssistant.settings.name })
    fireEvent.click(
      screen.getByRole("button", { name: en.personalAssistant.settings.uploadAvatar }),
    )
    fireEvent.click(await screen.findByRole("button", { name: "Confirm crop" }))
    fixture.revision++
    profile = { name: "New owner", avatar: null }
    rerender(<Harness />)
    await act(async () => resolve(new ArrayBuffer(3)))
    expect(fixture.call.mock.calls.some(([command]) => command === "save_avatar")).toBe(false)
  })

  it("saves its own name and preset, updates the header and greeting, and supports restoring the default avatar", async () => {
    render(<Harness />)
    const name = await screen.findByRole("textbox", { name: en.personalAssistant.settings.name })
    fireEvent.change(name, { target: { value: "  Mira  " } })
    fireEvent.click(screen.getByRole("button", { name: en.personalAssistant.settings.presets.bot }))
    fireEvent.click(screen.getByRole("button", { name: en.common.save }))
    await screen.findByText("I’m Mira", { selector: "h1" })
    expect(fixture.call).toHaveBeenCalledWith("update_personal_assistant_profile", {
      profile: { name: "Mira", avatar: "preset:bot" },
    })
    expect(
      fixture.call.mock.calls.some(
        ([command]) => command === "rename_session_cmd" || command === "update_agent_config",
      ),
    ).toBe(false)
    fireEvent.click(screen.getByRole("button", { name: en.personalAssistant.settings.resetAvatar }))
    fireEvent.submit(name.closest("form")!)
    await waitFor(() => expect(profile.avatar).toBeNull())
  })

  it("uses the shared crop/upload flow and persists the uploaded path only on Save", async () => {
    render(<Harness />)
    await screen.findByRole("textbox", { name: en.personalAssistant.settings.name })
    fireEvent.click(
      screen.getByRole("button", { name: en.personalAssistant.settings.uploadAvatar }),
    )
    fireEvent.click(await screen.findByRole("button", { name: "Confirm crop" }))
    await waitFor(() =>
      expect(document.querySelector('img[src="/api/avatars/avatar.png"]')).not.toBeNull(),
    )
    expect(profile.avatar).toBeNull()
    fireEvent.click(screen.getByRole("button", { name: en.common.save }))
    await waitFor(() => expect(profile.avatar).toBe("/isolated/avatars/avatar.png"))
    expect(fixture.call).toHaveBeenCalledWith("save_avatar", {
      data: [1, 2, 3],
      fileName: expect.stringMatching(/^assistant-.*\.png$/),
    })
  })

  it("keeps the draft after a failed save and prevents duplicate submissions", async () => {
    let reject!: (cause: Error) => void
    write = () =>
      new Promise((_, fail) => {
        reject = fail
      })
    render(<Harness />)
    const name = await screen.findByRole("textbox", { name: en.personalAssistant.settings.name })
    fireEvent.change(name, { target: { value: "Mira" } })
    fireEvent.submit(name.closest("form")!)
    fireEvent.submit(name.closest("form")!)
    expect(
      fixture.call.mock.calls.filter(
        ([command]) => command === "update_personal_assistant_profile",
      ),
    ).toHaveLength(1)
    await act(async () => reject(new Error("Disk unavailable")))
    expect(name).toHaveValue("Mira")
    expect(screen.getByRole("alert")).toHaveTextContent("Disk unavailable")
    expect(screen.getByRole("button", { name: en.common.saveFailed })).not.toBeDisabled()
  })

  it("discards a late save result when the owner connection changes", async () => {
    let resolve!: (value: AssistantProfile) => void
    write = () =>
      new Promise((done) => {
        resolve = done
      })
    const { rerender } = render(<Harness />)
    const name = await screen.findByRole("textbox", { name: en.personalAssistant.settings.name })
    fireEvent.change(name, { target: { value: "Old owner" } })
    fireEvent.submit(name.closest("form")!)
    fixture.revision++
    profile = { name: "New owner", avatar: null }
    rerender(<Harness />)
    await screen.findByText("I’m New owner", { selector: "h1" })
    await act(async () => resolve({ name: "Old owner", avatar: null }))
    expect(screen.getByRole("textbox", { name: en.personalAssistant.settings.name })).toHaveValue(
      "New owner",
    )
    expect(screen.queryByText("I’m Old owner", { selector: "h1" })).toBeNull()
  })
})
