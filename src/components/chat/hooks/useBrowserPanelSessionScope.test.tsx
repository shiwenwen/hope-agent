// @vitest-environment jsdom

import { useRef, useState } from "react"
import { act, cleanup, renderHook } from "@testing-library/react"
import { afterEach, expect, test, vi } from "vitest"

import { useBrowserPanelSessionScope } from "./useBrowserPanelSessionScope"

afterEach(cleanup)

function useTestPanel(sessionId: string | null, incognito = false) {
  const [visible, setVisible] = useState(false)
  const dismissedRef = useRef(false)
  const [closeFloating] = useState(() => vi.fn())
  const promote = useBrowserPanelSessionScope({
    sessionId, incognito, visible, setVisible, dismissedRef, closeFloating,
  })
  return { visible, setVisible, dismissedRef, closeFloating, promote }
}

test("main and side conversations preserve separate mirror visibility and dismissal", () => {
  const { result, rerender } = renderHook(({ id }) => useTestPanel(id), {
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
    result.current.dismissedRef.current = true
    result.current.setVisible(false)
  })
  rerender({ id: "side-2" })
  expect(result.current.dismissedRef.current).toBe(false)
  rerender({ id: "side-1" })
  expect(result.current.dismissedRef.current).toBe(true)
  expect(result.current.closeFloating).toHaveBeenCalledWith("browser")
})

test("draft promotion retains the mirror without leaking it to the next draft", () => {
  const { result, rerender } = renderHook(({ id }: { id: string | null }) => useTestPanel(id), {
    initialProps: { id: null as string | null },
  })
  act(() => {
    result.current.setVisible(true)
    result.current.promote("created")
  })
  rerender({ id: "created" })
  expect(result.current.visible).toBe(true)
  rerender({ id: null })
  expect(result.current.visible).toBe(false)
})

test("incognito mirror state is not restored after leaving its conversation", () => {
  const { result, rerender } = renderHook(({ id, incognito }) => useTestPanel(id, incognito), {
    initialProps: { id: "private", incognito: true },
  })
  act(() => {
    result.current.setVisible(true)
    result.current.dismissedRef.current = true
  })
  rerender({ id: "main", incognito: false })
  rerender({ id: "private", incognito: true })
  expect(result.current.visible).toBe(false)
  expect(result.current.dismissedRef.current).toBe(false)
})
