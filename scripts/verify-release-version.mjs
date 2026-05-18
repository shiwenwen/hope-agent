import { readFileSync } from "node:fs"
import path from "node:path"
import process from "node:process"

const rootDir = process.cwd()
const packageJsonPath = path.join(rootDir, "package.json")
const cargoTomlPath = path.join(rootDir, "src-tauri", "Cargo.toml")
const tauriConfigPath = path.join(rootDir, "src-tauri", "tauri.conf.json")
const cargoLockPath = path.join(rootDir, "Cargo.lock")
// ha-server ships the Docker image's hope-agent binary; its
// CARGO_PKG_VERSION must move with the desktop version (see
// scripts/sync-version.mjs).
const haServerCargoTomlPath = path.join(rootDir, "crates", "ha-server", "Cargo.toml")
// ha-core is the shared business-logic crate. Not user-facing, but kept
// in lockstep so the whole workspace reports one coherent version.
const haCoreCargoTomlPath = path.join(rootDir, "crates", "ha-core", "Cargo.toml")

const args = process.argv.slice(2)
let expectedTag = null

for (let i = 0; i < args.length; i += 1) {
  if (args[i] === "--tag") {
    expectedTag = args[i + 1] ?? null
    i += 1
  }
}

const packageJson = JSON.parse(readFileSync(packageJsonPath, "utf8"))
const tauriConfig = JSON.parse(readFileSync(tauriConfigPath, "utf8"))
const cargoToml = readFileSync(cargoTomlPath, "utf8")
const cargoVersionMatch = cargoToml.match(/^version = "(.*)"$/m)

if (!cargoVersionMatch) {
  console.error("[release:verify] could not read src-tauri/Cargo.toml version")
  process.exit(1)
}

const haServerCargoToml = readFileSync(haServerCargoTomlPath, "utf8")
const haServerVersionMatch = haServerCargoToml.match(/^version = "(.*)"$/m)
if (!haServerVersionMatch) {
  console.error("[release:verify] could not read crates/ha-server/Cargo.toml version")
  process.exit(1)
}

const haCoreCargoToml = readFileSync(haCoreCargoTomlPath, "utf8")
const haCoreVersionMatch = haCoreCargoToml.match(/^version = "(.*)"$/m)
if (!haCoreVersionMatch) {
  console.error("[release:verify] could not read crates/ha-core/Cargo.toml version")
  process.exit(1)
}

const cargoLock = readFileSync(cargoLockPath, "utf8")
const cargoLockHopeAgentMatch = cargoLock.match(/name = "hope-agent"\r?\nversion = "(.*)"/)
const cargoLockHaServerMatch = cargoLock.match(/name = "ha-server"\r?\nversion = "(.*)"/)
const cargoLockHaCoreMatch = cargoLock.match(/name = "ha-core"\r?\nversion = "(.*)"/)

if (!cargoLockHopeAgentMatch) {
  console.error("[release:verify] could not find hope-agent version in Cargo.lock")
  process.exit(1)
}
if (!cargoLockHaServerMatch) {
  console.error("[release:verify] could not find ha-server version in Cargo.lock")
  process.exit(1)
}
if (!cargoLockHaCoreMatch) {
  console.error("[release:verify] could not find ha-core version in Cargo.lock")
  process.exit(1)
}

const packageVersion = packageJson.version
const tauriVersion = tauriConfig.version
const cargoVersion = cargoVersionMatch[1]
const cargoLockVersion = cargoLockHopeAgentMatch[1]
const haServerVersion = haServerVersionMatch[1]
const haServerLockVersion = cargoLockHaServerMatch[1]
const haCoreVersion = haCoreVersionMatch[1]
const haCoreLockVersion = cargoLockHaCoreMatch[1]

const mismatches = [
  ["package.json", packageVersion],
  ["src-tauri/tauri.conf.json", tauriVersion],
  ["src-tauri/Cargo.toml", cargoVersion],
  ["Cargo.lock (hope-agent)", cargoLockVersion],
  ["crates/ha-server/Cargo.toml", haServerVersion],
  ["Cargo.lock (ha-server)", haServerLockVersion],
  ["crates/ha-core/Cargo.toml", haCoreVersion],
  ["Cargo.lock (ha-core)", haCoreLockVersion],
].filter(([, value], _, all) => value !== all[0][1])

if (mismatches.length > 0) {
  console.error("[release:verify] version mismatch detected:")
  console.error(`  package.json: ${packageVersion}`)
  console.error(`  src-tauri/tauri.conf.json: ${tauriVersion}`)
  console.error(`  src-tauri/Cargo.toml: ${cargoVersion}`)
  console.error(`  Cargo.lock (hope-agent): ${cargoLockVersion}`)
  console.error(`  crates/ha-server/Cargo.toml: ${haServerVersion}`)
  console.error(`  Cargo.lock (ha-server): ${haServerLockVersion}`)
  console.error(`  crates/ha-core/Cargo.toml: ${haCoreVersion}`)
  console.error(`  Cargo.lock (ha-core): ${haCoreLockVersion}`)
  process.exit(1)
}

if (expectedTag && expectedTag !== `v${packageVersion}`) {
  console.error(
    `[release:verify] tag ${expectedTag} does not match package version v${packageVersion}`,
  )
  process.exit(1)
}

console.log(`[release:verify] version OK: ${packageVersion}`)
if (expectedTag) {
  console.log(`[release:verify] tag OK: ${expectedTag}`)
}
