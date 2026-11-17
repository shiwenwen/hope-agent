// @vitest-environment jsdom
import { act, cleanup, renderHook } from "@testing-library/react"
import { afterEach, beforeEach, expect, test, vi } from "vitest"
import { usePersonalAssistantIdentity } from "./usePersonalAssistantIdentity"

const fixture = vi.hoisted(() => ({ revision: 1 }))
vi.mock("@/lib/transport-provider", () => ({
  useTransportRevision: () => fixture.revision,
  getTransportRevision: () => fixture.revision,
}))
beforeEach(() => {
  fixture.revision = 1
})
afterEach(cleanup)

test("an old connection's callback cannot project its profile or clear a new owner's identity", () => {
  const ownerA = { name: "Owner A", avatar: "/owner-a/private-avatar.png" }
  const ownerB = { name: "Owner B", avatar: "/owner-b/avatar.png" }
  const { result, rerender } = renderHook(usePersonalAssistantIdentity)
  const lateOwnerA = result.current.onProfileChange
  act(() => lateOwnerA(ownerA))
  expect(result.current.profile).toEqual(ownerA)

  fixture.revision = 2
  rerender()
  expect(result.current.profile).toBeNull()
  act(() => lateOwnerA(ownerA))
  expect(result.current.profile).toBeNull()

  act(() => result.current.onProfileChange(ownerB))
  expect(result.current.profile).toEqual(ownerB)
  act(() => lateOwnerA(null))
  expect(result.current.profile).toEqual(ownerB)
})
