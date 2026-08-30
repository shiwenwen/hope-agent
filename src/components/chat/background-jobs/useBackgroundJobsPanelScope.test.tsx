// @vitest-environment jsdom

import { act, cleanup, renderHook } from "@testing-library/react"
import { useState } from "react"
import { afterEach, expect, test } from "vitest"

import { useBackgroundJobsPanelScope } from "./useBackgroundJobsPanelScope"

afterEach(cleanup)

function useTestPanel(sessionId: string | null, incognito: boolean) {
  const [visible, setVisible] = useState(false)
  const scope = useBackgroundJobsPanelScope({ sessionId, incognito, visible, setVisible })
  return { visible, setVisible, ...scope }
}

test("main and side jobs retain independent visibility and dismissal", () => {
  const { result, rerender } = renderHook(({ id }) => useTestPanel(id, false), {
    initialProps: { id: "main" },
  })
  act(() => { result.current.dismissedRef.current = true })
  rerender({ id: "side-1" })
  expect(result.current.dismissedRef.current).toBe(false)
  act(() => { result.current.setVisible(true) })
  rerender({ id: "main" })
  expect(result.current.dismissedRef.current).toBe(true)
  expect(result.current.visible).toBe(false)
  rerender({ id: "side-1" })
  expect(result.current.visible).toBe(true)
  act(() => {
    result.current.previousRunningCountRef.current = 3
    result.current.suppressNextActivationRef.current = true
  })
  rerender({ id: "side-2" })
  expect(result.current.visible).toBe(false)
  expect(result.current.dismissedRef.current).toBe(false)
  expect(result.current.previousRunningCountRef.current).toBe(0)
  expect(result.current.suppressNextActivationRef.current).toBe(false)
})

test("draft promotion retains jobs visibility without leaking into the next draft", () => {
  const { result, rerender } = renderHook(
    ({ id }: { id: string | null }) => useTestPanel(id, false),
    { initialProps: { id: null as string | null } },
  )
  act(() => {
    result.current.setVisible(true)
    result.current.promote("created")
  })
  rerender({ id: "created" })
  expect(result.current.visible).toBe(true)
  rerender({ id: null })
  expect(result.current.visible).toBe(false)
})

test("incognito jobs panel visibility and dismissal are not cached", () => {
  const { result, rerender } = renderHook(
    ({ id, incognito }) => useTestPanel(id, incognito),
    { initialProps: { id: "private", incognito: true } },
  )
  act(() => {
    result.current.setVisible(true)
    result.current.dismissedRef.current = true
  })
  rerender({ id: "main", incognito: false })
  rerender({ id: "private", incognito: true })
  expect(result.current.visible).toBe(false)
  expect(result.current.dismissedRef.current).toBe(false)
})
