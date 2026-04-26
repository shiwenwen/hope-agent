/// <reference types="node" />
import { readFileSync } from "node:fs"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"
import { test, expect } from "vitest"

const indexCss = readFileSync(
  join(dirname(fileURLToPath(import.meta.url)), "index.css"),
  "utf8",
)

function ruleBodyFor(selectorPattern: RegExp): string {
  const match = indexCss.match(new RegExp(`${selectorPattern.source}\\s*\\{([^}]*)\\}`, "m"))
  return match?.[1] ?? ""
}

test("locks the webview document so only in-app panes can scroll", () => {
  const rootRule = ruleBodyFor(/html\s*,\s*body\s*,\s*#root/)

  expect(rootRule).toMatch(/height:\s*100%;/)
  expect(rootRule).toMatch(/overflow:\s*hidden;/)
  expect(rootRule).toMatch(/overscroll-behavior:\s*none;/)
})
