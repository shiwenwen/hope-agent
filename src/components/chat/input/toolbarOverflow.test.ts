import { test, expect } from "vitest"

import {
  CHAT_INPUT_INLINE_ADD_ACTIONS_CLASS,
  CHAT_INPUT_OVERFLOW_ACTION_IDS,
  CHAT_INPUT_OVERFLOW_BREAKPOINT_PX,
  CHAT_INPUT_OVERFLOW_MENU_CLASS,
} from "./toolbarOverflow.ts"
import * as toolbarOverflow from "./toolbarOverflow.ts"

test("groups add-style chat input actions behind the overflow menu", () => {
  expect(CHAT_INPUT_OVERFLOW_ACTION_IDS).toEqual([
    "attach-files",
    "working-dir",
    "slash-command",
    "incognito",
  ])
})

test("keeps overflow visibility classes static for Tailwind scanning", () => {
  expect(CHAT_INPUT_INLINE_ADD_ACTIONS_CLASS).toBe("contents max-[900px]:hidden")
  expect(CHAT_INPUT_OVERFLOW_MENU_CLASS).toBe("hidden max-[900px]:block")
  // JS-side breakpoint must mirror the Tailwind class so the matchMedia
  // auto-close stays in lockstep with the CSS toggle.
  expect(CHAT_INPUT_OVERFLOW_BREAKPOINT_PX).toBe(900)
})

test("shows the incognito preset action only before a session exists", () => {
  expect(typeof toolbarOverflow.shouldShowIncognitoPresetAction).toBe("function")
  const { shouldShowIncognitoPresetAction } = toolbarOverflow

  expect(shouldShowIncognitoPresetAction(null, true)).toBe(true)
  expect(shouldShowIncognitoPresetAction("session-1", true)).toBe(false)
  expect(shouldShowIncognitoPresetAction(null, false)).toBe(false)
})

test("filters overflow incognito action for existing sessions", () => {
  expect(typeof toolbarOverflow.getChatInputOverflowActionIds).toBe("function")
  const { getChatInputOverflowActionIds } = toolbarOverflow

  expect(getChatInputOverflowActionIds(null, true)).toEqual([
    "attach-files",
    "working-dir",
    "slash-command",
    "incognito",
  ])
  expect(getChatInputOverflowActionIds("session-1", true)).toEqual([
    "attach-files",
    "working-dir",
    "slash-command",
  ])
})
