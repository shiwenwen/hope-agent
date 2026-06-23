import { copyFileSync, existsSync, mkdirSync, readFileSync, rmSync } from "node:fs"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { LOCALE_MESSAGE_FILES } from "../extensions/chrome/scripts/locales.mjs"

// Stage the Chrome extension's RUNTIME files into the Tauri resources tree so
// release bundles ship the extension for local ("unpacked") install — the path
// `unpacked_extension_path()` resolves at runtime (see diagnostics.rs).
//
// Unlike the Web Store zip (extensions/chrome/scripts/package-webstore.mjs,
// which STRIPS `manifest.key`), this keeps `key`. The unpacked install must
// resolve to the fixed development extension id, because the native host's
// `allowed_origins` is derived from that id — so a locally-loaded extension can
// connect to the broker without a Web Store id ever being assigned.

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..")
const extensionDir = join(repoRoot, "extensions", "chrome")
const outDir = join(repoRoot, "src-tauri", "resources", "chrome-extension")

// Runtime file set — mirrors package-webstore.mjs's list (single source for the
// locale set is scripts/locales.mjs), minus the store-only manifest rewrite.
const RUNTIME_FILES = [
  "manifest.json",
  "service_worker.js",
  "popup.html",
  "popup.js",
  "icons/icon16.png",
  "icons/icon32.png",
  "icons/icon48.png",
  "icons/icon128.png",
  ...LOCALE_MESSAGE_FILES,
]

// Sanity: the bundled (unpacked) manifest MUST keep `key`. Without it Chrome
// assigns a path-derived random id, and the native host allowed_origins (built
// from the fixed id) won't match — the local extension would fail to connect.
const manifest = JSON.parse(readFileSync(join(extensionDir, "manifest.json"), "utf8"))
if (!manifest.key) {
  console.error(
    "[prepare-chrome-extension] manifest.json is missing `key`; an unpacked install would get an unstable id and fail to connect",
  )
  process.exit(1)
}

// Rebuild from scratch so renamed/removed source files never linger in the bundle.
rmSync(outDir, { recursive: true, force: true })
mkdirSync(outDir, { recursive: true })

let count = 0
for (const rel of RUNTIME_FILES) {
  const src = join(extensionDir, rel)
  if (!existsSync(src)) {
    console.error(`[prepare-chrome-extension] missing runtime file: ${src}`)
    process.exit(1)
  }
  const dest = join(outDir, rel)
  mkdirSync(dirname(dest), { recursive: true })
  copyFileSync(src, dest)
  count++
}

console.log(`[prepare-chrome-extension] staged ${count} files -> ${outDir}`)
