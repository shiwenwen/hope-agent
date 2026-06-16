import { readFileSync, writeFileSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..")
const files = [
  "node_modules/@codemirror/view/dist/index.js",
  "node_modules/@codemirror/view/dist/index.cjs",
]

const original =
  "if (window.EditContext && browser.android && view.constructor.EDIT_CONTEXT !== false &&"
const patched =
  "if (window.EditContext && (browser.android || window.__HOPE_AGENT_CODEMIRROR_EDIT_CONTEXT === true) && view.constructor.EDIT_CONTEXT !== false &&"

for (const relative of files) {
  const path = resolve(root, relative)
  const source = readFileSync(path, "utf8")
  if (source.includes(patched)) continue
  if (!source.includes(original)) {
    throw new Error(`Could not find CodeMirror EditContext gate in ${relative}`)
  }
  writeFileSync(path, source.replace(original, patched))
  console.log(`Patched ${relative} to allow Hope Agent scoped EditContext opt-in`)
}
