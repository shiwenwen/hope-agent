// @vitest-environment jsdom

import { useRef, useState } from "react"
import { act, cleanup, renderHook } from "@testing-library/react"
import { afterEach, expect, test, vi } from "vitest"

import { useMirrorPanelSessionScope } from "./useMirrorPanelSessionScope"

afterEach(cleanup)

const mirrorPanels = ["browser", "mac-control"] as const

function useTestPanel(panel: typeof mirrorPanels[number], sessionId: string | null, incognito = false) {
  const [visible, setVisible] = useState(false)
  const dismissedRef = useRef(false)
  const [closeFloating] = useState(() => vi.fn())
  const promote = useMirrorPanelSessionScope({
    panel, sessionId, incognito, visible, setVisible, dismissedRef, closeFloating,
  })
  return { visible, setVisible, dismissedRef, closeFloating, promote }
}

test.each(mirrorPanels)("%s: main and side conversations preserve separate visibility and dismissal", (panel) => {
  const { result, rerender } = renderHook(({ id }) => useTestPanel(panel, id), {
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
  expect(result.current.closeFloating).toHaveBeenCalledWith(panel)
})

test.each(mirrorPanels)("%s: draft promotion retains the mirror without leaking it to the next draft", (panel) => {
  const { result, rerender } = renderHook(({ id }: { id: string | null }) => useTestPanel(panel, id), {
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

test.each(mirrorPanels)("%s: incognito state is not restored after leaving its conversation", (panel) => {
  const { result, rerender } = renderHook(({ id, incognito }) => useTestPanel(panel, id, incognito), {
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

test("browser and Mac mirror caches remain independent while switching the same conversations", () => {
  const { result, rerender } = renderHook(({ id }) => ({
    browser: useTestPanel("browser", id),
    mac: useTestPanel("mac-control", id),
  }), { initialProps: { id: "main" } })
  act(() => {
    result.current.mac.dismissedRef.current = true
    result.current.browser.setVisible(true)
  })
  rerender({ id: "side" })
  expect(result.current.mac.dismissedRef.current).toBe(false)
  expect(result.current.browser.visible).toBe(false)
  act(() => {
    result.current.mac.setVisible(true)
    result.current.browser.dismissedRef.current = true
  })
  rerender({ id: "main" })
  expect(result.current.mac.dismissedRef.current).toBe(true)
  expect(result.current.mac.visible).toBe(false)
  expect(result.current.browser.dismissedRef.current).toBe(false)
  expect(result.current.browser.visible).toBe(true)
  rerender({ id: "side" })
  expect(result.current.mac.visible).toBe(true)
  expect(result.current.mac.dismissedRef.current).toBe(false)
  expect(result.current.browser.dismissedRef.current).toBe(true)
  expect(result.current.browser.visible).toBe(false)
})
