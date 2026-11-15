// @vitest-environment jsdom
import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, expect, it, vi } from "vitest"
import { useAssistantResearch, type AssistantFinding } from "./useAssistantResearch"

const fixture = vi.hoisted(() => ({
  revision: 1,
  call: vi.fn(),
  listeners: new Map<string, (payload: unknown) => void>(),
  listen: vi.fn((event: string, listener: (payload: unknown) => void) => {
    fixture.listeners.set(event, listener)
    return () => fixture.listeners.delete(event)
  }),
}))
vi.mock("@/lib/transport-provider", () => ({
  useTransport: () => fixture,
  useTransportRevision: () => fixture.revision,
  getTransportRevision: () => fixture.revision,
}))
const finding: AssistantFinding = {
  id: "finding-1",
  revision: 1,
  fact: "PRIVATE_FINDING_BODY",
  inference: "",
  uncertainty: "",
  prepared: "Option A",
  references: [],
  createdAt: "2026-11-15T12:00:00Z",
  viewedAt: null,
}
function reads(command: string): unknown {
  if (command === "list_personal_assistant_findings") return [finding]
  if (command === "get_personal_assistant_policy")
    return {
      requestedRevision: 1,
      appliedRevision: 1,
      pending: false,
      failed: false,
      settings: {
        researchEnabled: true,
        paused: false,
        timezone: null,
        noticeRoute: "none",
        quietStartHour: 22,
        quietEndHour: 8,
      },
    }
  return []
}
beforeEach(() => {
  fixture.revision = 1
  fixture.listeners.clear()
  fixture.call.mockReset().mockImplementation(async (command: string) => reads(command))
})
afterEach(cleanup)

it("discards a late private body when Hope is hidden or its connection changes", async () => {
  let finish: ((value: unknown) => void) | undefined
  fixture.call.mockImplementation((command: string) =>
    command === "list_personal_assistant_findings"
      ? new Promise((resolve) => {
          finish = resolve
        })
      : Promise.resolve(reads(command)),
  )
  const { result, rerender } = renderHook(({ enabled }) => useAssistantResearch(enabled), {
    initialProps: { enabled: true },
  })
  rerender({ enabled: false })
  await act(async () => {
    finish?.([finding])
    await Promise.resolve()
  })
  expect(result.current.findings).toEqual([])
  fixture.revision++
  fixture.call.mockImplementation(async (command: string) =>
    command === "list_personal_assistant_findings" ? [] : reads(command),
  )
  rerender({ enabled: true })
  await waitFor(() => expect(result.current.loading).toBe(false))
  expect(result.current.findings).toEqual([])
})

it("clears revoked bodies immediately and never restores a stale in-flight response", async () => {
  const { result } = renderHook(() => useAssistantResearch(true))
  await waitFor(() => expect(result.current.findings).toHaveLength(1))
  let finish: ((value: unknown) => void) | undefined
  fixture.call.mockImplementation((command: string) =>
    command === "list_personal_assistant_findings"
      ? new Promise((resolve) => {
          finish = resolve
        })
      : Promise.resolve(reads(command)),
  )
  let pending: Promise<void>
  act(() => {
    pending = result.current.refresh()
  })
  act(() => {
    fixture.listeners.get("personal_assistant:research_changed")?.({ invalidate: true })
  })
  expect(result.current.findings).toEqual([])
  await act(async () => {
    finish?.([finding])
    await pending
  })
  expect(result.current.findings).toEqual([])
})

it("reuses an ambiguous owner request identity only for the same action and connection", async () => {
  const sent: Array<{ requestKey: string; question: string }> = []
  fixture.call.mockImplementation(
    async (command: string, args: { requestKey: string; question: string }) => {
      if (command !== "ask_personal_assistant_finding") return reads(command)
      sent.push(args)
      if (sent.length === 1) throw new Error("Reply lost after acceptance")
      return { id: "receipt", state: "pending", reason: null }
    },
  )
  const { result, rerender } = renderHook(() => useAssistantResearch(true))
  await waitFor(() => expect(result.current.loading).toBe(false))
  await expect(result.current.askFinding("finding-1", 1, "Prepare another option")).rejects.toThrow(
    "Reply lost",
  )
  await act(async () => {
    await result.current.askFinding("finding-1", 1, "Prepare another option")
  })
  expect(sent[0].requestKey).toBe(sent[1].requestKey)
  expect(sent[0].requestKey).toMatch(/^[0-9a-f-]{36}$/)
  await act(async () => {
    await result.current.askFinding("finding-1", 1, "A different question")
  })
  expect(sent[2].requestKey).not.toBe(sent[1].requestKey)
  fixture.revision++
  rerender()
  await waitFor(() => expect(result.current.loading).toBe(false))
  await act(async () => {
    await result.current.askFinding("finding-1", 1, "Prepare another option")
  })
  expect(sent[3].requestKey).not.toBe(sent[1].requestKey)
})
