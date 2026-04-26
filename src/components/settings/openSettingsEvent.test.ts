import { test, expect } from "vitest"

import { parseOpenSettingsSection } from "./openSettingsEvent.ts"

test("reads the target settings section from an open-settings payload", () => {
  expect(parseOpenSettingsSection({ section: "about" })).toBe("about")
})

test("ignores missing or unknown open-settings payload sections", () => {
  expect(parseOpenSettingsSection(undefined)).toBeUndefined()
  expect(parseOpenSettingsSection({})).toBeUndefined()
  expect(parseOpenSettingsSection({ section: "missing" })).toBeUndefined()
  expect(parseOpenSettingsSection({ section: 123 })).toBeUndefined()
})
