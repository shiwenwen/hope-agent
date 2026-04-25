/// <reference types="node" />

import assert from "node:assert/strict"
import { test } from "node:test"

import { parseOpenSettingsSection } from "./openSettingsEvent.ts"

test("reads the target settings section from an open-settings payload", () => {
  assert.equal(parseOpenSettingsSection({ section: "about" }), "about")
})

test("ignores missing or unknown open-settings payload sections", () => {
  assert.equal(parseOpenSettingsSection(undefined), undefined)
  assert.equal(parseOpenSettingsSection({}), undefined)
  assert.equal(parseOpenSettingsSection({ section: "missing" }), undefined)
  assert.equal(parseOpenSettingsSection({ section: 123 }), undefined)
})
