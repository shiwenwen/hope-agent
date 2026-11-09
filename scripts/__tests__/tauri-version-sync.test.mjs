import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs"
import os from "node:os"
import path from "node:path"
import test from "node:test"

const pairs = [
  ["tauri", "@tauri-apps/api", "2.12.4"],
  ["tauri-plugin-dialog", "@tauri-apps/plugin-dialog", "2.8.3"],
  ["tauri-plugin-notification", "@tauri-apps/plugin-notification", "2.7.0"],
  ["tauri-plugin-process", "@tauri-apps/plugin-process", "2.4.0"],
  ["tauri-plugin-updater", "@tauri-apps/plugin-updater", "2.14.0"],
]

function check({ resolved = {}, declared = {}, missingCrate, missingImporter } = {}) {
  const root = mkdtempSync(path.join(os.tmpdir(), "hope-tauri-version-sync-"))
  try {
    mkdirSync(path.join(root, "scripts"))
    copyFileSync(new URL("../verify-tauri-version-sync.mjs", import.meta.url), path.join(root, "scripts/verify-tauri-version-sync.mjs"))
    writeFileSync(path.join(root, "package.json"), JSON.stringify({ dependencies: Object.fromEntries(pairs.map(([, pkg, version]) => [pkg, declared[pkg] ?? `^${version}`])) }))
    writeFileSync(path.join(root, "Cargo.lock"), pairs.filter(([crate]) => crate !== missingCrate).map(([crate, , version]) => `[[package]]\nname = "${crate}"\nversion = "${version}"\n`).join("\n"))
    writeFileSync(path.join(root, "pnpm-lock.yaml"), "importers:\n  .:\n    dependencies:\n" + pairs.filter(([, pkg]) => pkg !== missingImporter).map(([, pkg, version]) => `      '${pkg}':\n        specifier: ${declared[pkg] ?? `^${version}`}\n        version: ${resolved[pkg] ?? version}\n`).join(""))
    return spawnSync(process.execPath, [path.join(root, "scripts/verify-tauri-version-sync.mjs")], { encoding: "utf8" })
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
}

test("matching pairs accept patch differences and peer suffixes", () => {
  const result = check({ resolved: { "@tauri-apps/plugin-dialog": "2.8.9", "@tauri-apps/api": "2.12.5(typescript@6.0.2)" } })
  assert.equal(result.status, 0, result.stderr)
  assert.match(result.stdout, /5 Rust\/npm package pairs/)
})

test("each plugin mismatch fails even when the base API and other plugins match", () => {
  for (const [crate, pkg] of pairs.slice(1)) {
    const result = check({ resolved: { [pkg]: "2.3.1" } })
    assert.equal(result.status, 1, pkg)
    assert.ok(result.stderr.includes(crate) && result.stderr.includes(pkg), result.stderr)
  }
})

test("the resolved lock version wins over a compatible declared floor", () => {
  const result = check({ resolved: { "@tauri-apps/api": "2.13.0" } })
  assert.equal(result.status, 1)
  assert.match(result.stderr, /pnpm-lock.yaml 2.13.0/)
})

test("missing native counterparts and importer entries fail closed", () => {
  for (const missing of [{ missingCrate: "tauri-plugin-process" }, { missingImporter: "@tauri-apps/plugin-notification" }]) {
    const result = check(missing)
    assert.equal(result.status, 1)
    assert.match(result.stderr, /has no/)
  }
})
