// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { TooltipProvider } from "@/components/ui/tooltip"
import type { KbAttachment, KnowledgeBaseMeta } from "@/types/knowledge"
import KnowledgePicker from "./input/KnowledgePicker"
import ProjectKnowledgeSection from "./project/ProjectKnowledgeSection"

const transport = vi.hoisted(() => ({
  call: vi.fn(),
  listen: vi.fn(() => () => {}),
}))

vi.mock("@/lib/transport-provider", () => ({ getTransport: () => transport }))
vi.mock("react-i18next", () => {
  const t = (key: string) => key
  return {
    initReactI18next: { type: "3rdParty", init: () => {} },
    useTranslation: () => ({ t }),
  }
})

let vault: KnowledgeBaseMeta
let attachments: KbAttachment[]

beforeEach(() => {
  vault = {
    id: "vault-1",
    name: "External notes",
    rootDir: "/synthetic-vault",
    external: true,
    allowExternalWrites: true,
    externalRawSync: "disabled",
    archived: false,
    createdAt: 1,
    updatedAt: 1,
    noteCount: 2,
  }
  attachments = []
  transport.call.mockImplementation((name: string) => {
    if (name === "list_kbs_cmd") return Promise.resolve([vault])
    if (name === "list_session_kbs_cmd" || name === "list_project_kbs_cmd") {
      return Promise.resolve(attachments)
    }
    return Promise.resolve(null)
  })
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

async function openSurface(surface: "session" | "project") {
  render(
    <TooltipProvider>
      {surface === "session" ? (
        <KnowledgePicker sessionId="s1" projectId="p1" variant="menu" />
      ) : (
        <ProjectKnowledgeSection projectId="p1" />
      )}
    </TooltipProvider>,
  )
  if (surface === "session") {
    fireEvent.click(screen.getByRole("button", { name: "knowledge.picker.title" }))
  }
  await screen.findByText(vault.name)
}

async function chooseWrite(surface: "session" | "project") {
  fireEvent.click(screen.getByRole("radio", { name: "knowledge.picker.accessWrite" }))
  await waitFor(() => {
    expect(transport.call).toHaveBeenCalledWith(`attach_${surface}_kb_cmd`, {
      [surface === "session" ? "sessionId" : "projectId"]: surface === "session" ? "s1" : "p1",
      kbId: vault.id,
      access: "write",
    })
  })
}

describe.each(["session", "project"] as const)("%s knowledge write grants", (surface) => {
  it("lets the owner explicitly grant write after enabling external writes", async () => {
    await openSurface(surface)
    expect(transport.call.mock.calls.some(([name]) => name.startsWith("attach_"))).toBe(false)
    await chooseWrite(surface)
  })

  it("keeps an external vault without write opt-in limited to read", async () => {
    vault.allowExternalWrites = false
    await openSurface(surface)
    expect(screen.queryByRole("radio", { name: "knowledge.picker.accessWrite" })).toBeNull()
    fireEvent.click(screen.getByRole("radio", { name: "knowledge.picker.accessRead" }))
    await waitFor(() => {
      expect(transport.call).toHaveBeenCalledWith(`attach_${surface}_kb_cmd`, {
        [surface === "session" ? "sessionId" : "projectId"]: surface === "session" ? "s1" : "p1",
        kbId: vault.id,
        access: "read",
      })
    })
  })

  it("preserves write grants for internal spaces without the external opt-in", async () => {
    vault.external = false
    vault.rootDir = null
    vault.allowExternalWrites = false
    await openSurface(surface)
    await chooseWrite(surface)
  })
})

it("stages an opted-in external write grant for a new session without a backend mutation", async () => {
  const onDraftAttachChange = vi.fn()
  render(
    <TooltipProvider>
      <KnowledgePicker sessionId={null} variant="menu" onDraftAttachChange={onDraftAttachChange} />
    </TooltipProvider>,
  )
  fireEvent.click(screen.getByRole("button", { name: "knowledge.picker.title" }))
  fireEvent.click(await screen.findByRole("radio", { name: "knowledge.picker.accessWrite" }))
  expect(onDraftAttachChange).toHaveBeenCalledWith([{ kbId: vault.id, access: "write" }])
  expect(transport.call.mock.calls.some(([name]) => name.startsWith("attach_"))).toBe(false)
})

it.each(["read", "write"] as const)(
  "keeps inherited project %s grants immutable in a session",
  async (access) => {
    attachments = [{ ...vault, access, via: "project" }]
    await openSurface("session")
    const selected = screen.getByRole("radio", {
      name: access === "write" ? "knowledge.picker.accessWrite" : "knowledge.picker.accessRead",
    })
    expect(selected.getAttribute("aria-checked")).toBe("true")
    for (const radio of screen.getAllByRole("radio")) {
      expect((radio as HTMLButtonElement).disabled).toBe(true)
      fireEvent.click(radio)
    }
    expect(transport.call.mock.calls.some(([name]) => /^(attach|detach)_/.test(name))).toBe(false)
  },
)
