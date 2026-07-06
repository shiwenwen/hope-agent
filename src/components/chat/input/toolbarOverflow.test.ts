import { test, expect } from "vitest"

import {
  CHAT_INPUT_INLINE_ADD_ACTIONS_CLASS,
  CHAT_INPUT_OVERFLOW_ACTION_IDS,
  CHAT_INPUT_OVERFLOW_MENU_CLASS,
  CHAT_INPUT_TOOLBAR_GROUP_WIDTH_FALLBACKS,
  CHAT_INPUT_TOOLBAR_MAX_COLLAPSE_LEVEL,
  clampChatInputToolbarCollapseLevel,
  getChatInputToolbarFlags,
} from "./toolbarOverflow.ts"
import * as toolbarOverflow from "./toolbarOverflow.ts"

test("groups add-style chat input actions behind the overflow menu", () => {
  expect(CHAT_INPUT_OVERFLOW_ACTION_IDS).toEqual(["working-dir", "attach-files", "slash-command"])
})

test("keeps overflow visibility classes static for Tailwind scanning", () => {
  expect(CHAT_INPUT_INLINE_ADD_ACTIONS_CLASS).toBe("flex items-center gap-1 shrink-0")
  expect(CHAT_INPUT_OVERFLOW_MENU_CLASS).toBe("hidden")
})

test("maps smart toolbar collapse levels to progressive visibility flags", () => {
  expect(CHAT_INPUT_TOOLBAR_MAX_COLLAPSE_LEVEL).toBe(4)
  expect(clampChatInputToolbarCollapseLevel(-1)).toBe(0)
  expect(clampChatInputToolbarCollapseLevel(99)).toBe(4)
  expect(getChatInputToolbarFlags(0)).toEqual({
    toolbarCompact: false,
    toolbarTight: false,
    sandboxCollapsed: false,
    permissionCollapsed: false,
  })
  expect(getChatInputToolbarFlags(2)).toEqual({
    toolbarCompact: true,
    toolbarTight: true,
    sandboxCollapsed: false,
    permissionCollapsed: false,
  })
  expect(getChatInputToolbarFlags(4)).toEqual({
    toolbarCompact: true,
    toolbarTight: true,
    sandboxCollapsed: true,
    permissionCollapsed: true,
  })
})

test("keeps conservative width fallbacks for first smart toolbar measurement", () => {
  expect(CHAT_INPUT_TOOLBAR_GROUP_WIDTH_FALLBACKS.addActions).toBeGreaterThan(
    CHAT_INPUT_TOOLBAR_GROUP_WIDTH_FALLBACKS.overflowTrigger,
  )
  expect(CHAT_INPUT_TOOLBAR_GROUP_WIDTH_FALLBACKS.semanticModes).toBeGreaterThan(0)
})

test("returns overflow actions for the compact input toolbar", () => {
  expect(typeof toolbarOverflow.getChatInputOverflowActionIds).toBe("function")
  const { getChatInputOverflowActionIds } = toolbarOverflow

  expect(getChatInputOverflowActionIds()).toEqual(["working-dir", "attach-files", "slash-command"])
})
