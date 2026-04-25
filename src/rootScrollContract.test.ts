/// <reference types="node" />

import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import { test } from "node:test"

const indexCss = readFileSync(new URL("./index.css", import.meta.url), "utf8")

function ruleBodyFor(selectorPattern: RegExp): string {
  const match = indexCss.match(new RegExp(`${selectorPattern.source}\\s*\\{([^}]*)\\}`, "m"))
  return match?.[1] ?? ""
}

test("locks the webview document so only in-app panes can scroll", () => {
  const rootRule = ruleBodyFor(/html\s*,\s*body\s*,\s*#root/)

  assert.match(rootRule, /height:\s*100%;/)
  assert.match(rootRule, /overflow:\s*hidden;/)
  assert.match(rootRule, /overscroll-behavior:\s*none;/)
})
