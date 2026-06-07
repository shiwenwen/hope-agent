import { test, expect } from "vitest"

import {
  CHAT_INPUT_INLINE_ADD_ACTIONS_CLASS,
  CHAT_INPUT_OVERFLOW_ACTION_IDS,
  CHAT_INPUT_OVERFLOW_BREAKPOINT_PX,
  CHAT_INPUT_OVERFLOW_MENU_CLASS,
  CHAT_INPUT_STACKED_TOOLBAR_BREAKPOINT_PX,
} from "./toolbarOverflow.ts"
import * as toolbarOverflow from "./toolbarOverflow.ts"

test("groups add-style chat input actions behind the overflow menu", () => {
  expect(CHAT_INPUT_OVERFLOW_ACTION_IDS).toEqual([
    "working-dir",
    "attach-files",
    "slash-command",
  ])
})

test("keeps overflow visibility classes static for Tailwind scanning", () => {
  expect(CHAT_INPUT_INLINE_ADD_ACTIONS_CLASS).toBe("contents")
  expect(CHAT_INPUT_OVERFLOW_MENU_CLASS).toBe("hidden")
  // JS-side breakpoint is measured against the input container width, so
  // right-side panels can trigger the compact toolbar without resizing window.
  expect(CHAT_INPUT_OVERFLOW_BREAKPOINT_PX).toBe(900)
  expect(CHAT_INPUT_STACKED_TOOLBAR_BREAKPOINT_PX).toBe(520)
})

test("returns overflow actions for the compact input toolbar", () => {
  expect(typeof toolbarOverflow.getChatInputOverflowActionIds).toBe("function")
  const { getChatInputOverflowActionIds } = toolbarOverflow

  expect(getChatInputOverflowActionIds()).toEqual([
    "working-dir",
    "attach-files",
    "slash-command",
  ])
})
