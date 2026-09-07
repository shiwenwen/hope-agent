// @vitest-environment jsdom

import { useState } from "react"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, expect, it, vi } from "vitest"
import AuthProfileEditor from "./AuthProfileEditor"
import type { AuthProfile } from "./types"

vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }))
afterEach(cleanup)

it("binds a workspace to one key without changing its credentials or sibling profiles", () => {
  const updates = vi.fn()
  const initial: AuthProfile[] = [
    { id: "one", label: "one", apiKey: "synthetic-one", enabled: true },
    { id: "two", label: "two", apiKey: "synthetic-two", enabled: true },
  ]
  function Editor() {
    const [profiles, setProfiles] = useState(initial)
    return (
      <AuthProfileEditor
        anthropic
        profiles={profiles}
        onChange={(next) => {
          setProfiles(next)
          updates(next)
        }}
      />
    )
  }
  render(<Editor />)
  fireEvent.click(screen.getAllByRole("switch", { name: "authProfiles.multiWorkspace" })[0])
  expect(updates.mock.lastCall?.[0][0].anthropicWorkspaceId).toBe("")
  fireEvent.change(screen.getByRole("textbox", { name: "authProfiles.workspaceId" }), {
    target: { value: "wrkspc_A" },
  })
  expect(updates.mock.lastCall?.[0]).toEqual([
    { ...initial[0], anthropicWorkspaceId: "wrkspc_A" },
    initial[1],
  ])
  fireEvent.click(screen.getAllByRole("switch", { name: "authProfiles.multiWorkspace" })[0])
  expect(updates.mock.lastCall?.[0][0].anthropicWorkspaceId).toBeUndefined()
  expect(screen.queryByRole("textbox", { name: "authProfiles.workspaceId" })).toBeNull()
})

it("allows clearing an existing binding after changing API type", () => {
  render(
    <AuthProfileEditor
      profiles={[
        {
          id: "one",
          label: "",
          apiKey: "synthetic",
          enabled: true,
          anthropicWorkspaceId: "wrkspc_A",
        },
      ]}
      onChange={vi.fn()}
    />,
  )
  expect(screen.getByRole("switch", { name: "authProfiles.multiWorkspace" })).toBeTruthy()
})
