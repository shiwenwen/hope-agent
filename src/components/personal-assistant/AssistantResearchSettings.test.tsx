// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterAll, afterEach, beforeAll, expect, it, vi } from "vitest"
import { AssistantResearchSettings } from "./AssistantResearchSettings"
import type { AssistantResearchState } from "./useAssistantResearch"
import en from "@/i18n/locales/en.json"
vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) =>
      key
        .split(".")
        .reduce<unknown>(
          (value, segment) =>
            value && typeof value === "object"
              ? (value as Record<string, unknown>)[segment]
              : undefined,
          en,
        ) ?? key,
  }),
}))
beforeAll(() =>
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe = vi.fn()
      unobserve = vi.fn()
      disconnect = vi.fn()
    },
  ),
)
afterAll(() => vi.unstubAllGlobals())
afterEach(cleanup)

it("keeps a received policy pending until Primary confirms the exact revision", async () => {
  const settings = {
    researchEnabled: false,
    paused: false,
    timezone: null,
    noticeRoute: "none" as const,
    quietStartHour: 22,
    quietEndHour: 8,
  }
  const savePolicy = vi.fn(async () => ({
    requestedRevision: 2,
    appliedRevision: 1,
    settings: { ...settings, researchEnabled: true },
    pending: true,
    failed: false,
  }))
  const state = {
    policy: { requestedRevision: 1, appliedRevision: 1, settings, pending: false, failed: false },
    sources: [],
    findings: [],
    activity: [],
    error: null,
    savePolicy,
  } as unknown as AssistantResearchState
  const { rerender } = render(<AssistantResearchSettings state={state} sessions={[]} />)
  fireEvent.click(screen.getByRole("switch", { name: "Find changes in shared material" }))
  fireEvent.click(screen.getByRole("button", { name: "Save" }))
  await waitFor(() => expect(savePolicy).toHaveBeenCalledTimes(1))
  expect(screen.queryByRole("button", { name: "Saved" })).toBeNull()
  await waitFor(() => expect(screen.getByText("Applying settings…")).toBeTruthy())
  expect(screen.queryByRole("button", { name: "Save failed" })).toBeNull()
  rerender(
    <AssistantResearchSettings
      state={{ ...state, policy: { ...state.policy!, requestedRevision: 2, pending: true } }}
      sessions={[]}
    />,
  )
  expect(screen.queryByRole("button", { name: "Saved" })).toBeNull()
  rerender(
    <AssistantResearchSettings
      state={{
        ...state,
        policy: { ...state.policy!, requestedRevision: 2, appliedRevision: 2, pending: false },
      }}
      sessions={[]}
    />,
  )
  await waitFor(() => expect(screen.getByRole("button", { name: "Saved" })).toBeTruthy())
})
