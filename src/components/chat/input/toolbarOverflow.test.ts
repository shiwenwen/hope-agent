/// <reference types="node" />

import assert from "node:assert/strict"
import { test } from "node:test"

import {
  CHAT_INPUT_INLINE_ADD_ACTIONS_CLASS,
  CHAT_INPUT_OVERFLOW_ACTION_IDS,
  CHAT_INPUT_OVERFLOW_MENU_CLASS,
} from "./toolbarOverflow.ts"

test("groups add-style chat input actions behind the overflow menu", () => {
  assert.deepEqual(CHAT_INPUT_OVERFLOW_ACTION_IDS, [
    "attach-files",
    "working-dir",
    "slash-command",
    "incognito",
  ])
})

test("keeps overflow visibility classes static for Tailwind scanning", () => {
  assert.equal(CHAT_INPUT_INLINE_ADD_ACTIONS_CLASS, "contents max-[900px]:hidden")
  assert.equal(CHAT_INPUT_OVERFLOW_MENU_CLASS, "hidden max-[900px]:block")
})
