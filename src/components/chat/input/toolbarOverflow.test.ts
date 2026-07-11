import { test, expect } from "vitest"

import {
  CHAT_INPUT_INLINE_ADD_ACTIONS_CLASS,
  CHAT_INPUT_OVERFLOW_ACTION_IDS,
  CHAT_INPUT_OVERFLOW_MENU_CLASS,
  CHAT_INPUT_TOOLBAR_FIT_BUFFER_PX,
  CHAT_INPUT_TOOLBAR_GROUP_WIDTH_FALLBACKS,
  CHAT_INPUT_TOOLBAR_MAX_COLLAPSE_LEVEL,
  clampChatInputToolbarCollapseLevel,
  estimateChatInputToolbarLevelWidths,
  getChatInputToolbarFlags,
  resolveChatInputToolbarCollapseLevel,
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

test("estimates all toolbar collapse tier widths from the currently visible tier", () => {
  const widths = CHAT_INPUT_TOOLBAR_GROUP_WIDTH_FALLBACKS

  expect(
    estimateChatInputToolbarLevelWidths({
      currentLevel: 2,
      visibleWidth: 420,
      widths,
    }),
  ).toEqual([836, 760, 420, 270, 135])
})

test("resolves the toolbar collapse tier directly from available width", () => {
  const widths = CHAT_INPUT_TOOLBAR_GROUP_WIDTH_FALLBACKS

  expect(
    resolveChatInputToolbarCollapseLevel({
      currentLevel: 0,
      availableWidth: 836 + CHAT_INPUT_TOOLBAR_FIT_BUFFER_PX,
      visibleWidth: 836,
      widths,
    }),
  ).toBe(0)

  expect(
    resolveChatInputToolbarCollapseLevel({
      currentLevel: 0,
      availableWidth: 500,
      visibleWidth: 836,
      widths,
    }),
  ).toBe(2)

  expect(
    resolveChatInputToolbarCollapseLevel({
      currentLevel: 0,
      availableWidth: 240,
      visibleWidth: 836,
      widths,
    }),
  ).toBe(4)
})

test("keeps an expansion buffer so compact toolbar tiers do not jitter", () => {
  const widths = CHAT_INPUT_TOOLBAR_GROUP_WIDTH_FALLBACKS

  expect(
    resolveChatInputToolbarCollapseLevel({
      currentLevel: 2,
      availableWidth: 700,
      visibleWidth: 420,
      widths,
    }),
  ).toBe(2)

  expect(
    resolveChatInputToolbarCollapseLevel({
      currentLevel: 2,
      availableWidth: 790,
      visibleWidth: 420,
      widths,
    }),
  ).toBe(1)
})
