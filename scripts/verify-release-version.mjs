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
// ha-base is the foundation layer split out of ha-core. sync-version bumps it,
// so the release gate must verify it too — otherwise a drifted ha-base manifest
// or lock entry would sail through `pnpm release:verify`.
const haBaseCargoTomlPath = path.join(rootDir, "crates", "ha-base", "Cargo.toml")
// ha-config-schema holds AppConfig's wire-type closure, split out of ha-core.
// Same rule: sync-version bumps it, the release gate verifies it.
const haConfigSchemaCargoTomlPath = path.join(rootDir, "crates", "ha-config-schema", "Cargo.toml")
// ha-browser-host ships in desktop bundles and bare-binary archives
// (updater extra_binaries) and reports hostVersion from CARGO_PKG_VERSION.
const browserHostCargoTomlPath = path.join(rootDir, "crates", "ha-browser-host", "Cargo.toml")
const haEvalCargoTomlPath = path.join(rootDir, "crates", "ha-eval", "Cargo.toml")
const haUpdaterCargoTomlPath = path.join(rootDir, "crates", "ha-updater", "Cargo.toml")
const haWeatherCargoTomlPath = path.join(rootDir, "crates", "ha-weather", "Cargo.toml")
const haAcpCargoTomlPath = path.join(rootDir, "crates", "ha-acp", "Cargo.toml")
const haMacCargoTomlPath = path.join(rootDir, "crates", "ha-mac", "Cargo.toml")
const haDesignCargoTomlPath = path.join(rootDir, "crates", "ha-design", "Cargo.toml")
const haBrowserCargoTomlPath = path.join(rootDir, "crates", "ha-browser", "Cargo.toml")
const haVcsCargoTomlPath = path.join(rootDir, "crates", "ha-vcs", "Cargo.toml")
const haMcpCargoTomlPath = path.join(rootDir, "crates", "ha-mcp", "Cargo.toml")
const haPetCargoTomlPath = path.join(rootDir, "crates", "ha-pet", "Cargo.toml")

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

const haBaseCargoToml = readFileSync(haBaseCargoTomlPath, "utf8")
const haBaseVersionMatch = haBaseCargoToml.match(/^version = "(.*)"$/m)
if (!haBaseVersionMatch) {
  console.error("[release:verify] could not read crates/ha-base/Cargo.toml version")
  process.exit(1)
}

const haConfigSchemaCargoToml = readFileSync(haConfigSchemaCargoTomlPath, "utf8")
const haConfigSchemaVersionMatch = haConfigSchemaCargoToml.match(/^version = "(.*)"$/m)
if (!haConfigSchemaVersionMatch) {
  console.error("[release:verify] could not read crates/ha-config-schema/Cargo.toml version")
  process.exit(1)
}

const browserHostCargoToml = readFileSync(browserHostCargoTomlPath, "utf8")
const browserHostVersionMatch = browserHostCargoToml.match(/^version = "(.*)"$/m)
if (!browserHostVersionMatch) {
  console.error("[release:verify] could not read crates/ha-browser-host/Cargo.toml version")
  process.exit(1)
}

const haEvalCargoToml = readFileSync(haEvalCargoTomlPath, "utf8")
const haEvalVersionMatch = haEvalCargoToml.match(/^version = "(.*)"$/m)
if (!haEvalVersionMatch) {
  console.error("[release:verify] could not read crates/ha-eval/Cargo.toml version")
  process.exit(1)
}

const haUpdaterCargoToml = readFileSync(haUpdaterCargoTomlPath, "utf8")
const haUpdaterVersionMatch = haUpdaterCargoToml.match(/^version = "(.*)"$/m)
if (!haUpdaterVersionMatch) {
  console.error("[release:verify] could not read crates/ha-updater/Cargo.toml version")
  process.exit(1)
}

const haWeatherCargoToml = readFileSync(haWeatherCargoTomlPath, "utf8")
const haWeatherVersionMatch = haWeatherCargoToml.match(/^version = "(.*)"$/m)
if (!haWeatherVersionMatch) {
  console.error("[release:verify] could not read crates/ha-weather/Cargo.toml version")
  process.exit(1)
}

const haAcpCargoToml = readFileSync(haAcpCargoTomlPath, "utf8")
const haAcpVersionMatch = haAcpCargoToml.match(/^version = "(.*)"$/m)
if (!haAcpVersionMatch) {
  console.error("[release:verify] could not read crates/ha-acp/Cargo.toml version")
  process.exit(1)
}

const haMacCargoToml = readFileSync(haMacCargoTomlPath, "utf8")
const haMacVersionMatch = haMacCargoToml.match(/^version = "(.*)"$/m)
if (!haMacVersionMatch) {
  console.error("[release:verify] could not read crates/ha-mac/Cargo.toml version")
  process.exit(1)
}

const haDesignCargoToml = readFileSync(haDesignCargoTomlPath, "utf8")
const haDesignVersionMatch = haDesignCargoToml.match(/^version = "(.*)"$/m)
if (!haDesignVersionMatch) {
  console.error("[release:verify] could not read crates/ha-design/Cargo.toml version")
  process.exit(1)
}

const haBrowserCargoToml = readFileSync(haBrowserCargoTomlPath, "utf8")
const haBrowserVersionMatch = haBrowserCargoToml.match(/^version = "(.*)"$/m)
if (!haBrowserVersionMatch) {
  console.error("[release:verify] could not read crates/ha-browser/Cargo.toml version")
  process.exit(1)
}

const haMcpCargoToml = readFileSync(haMcpCargoTomlPath, "utf8")
const haMcpVersionMatch = haMcpCargoToml.match(/^version = "(.*)"$/m)
if (!haMcpVersionMatch) {
  console.error("[release:verify] could not read crates/ha-mcp/Cargo.toml version")
  process.exit(1)
}

const haPetCargoToml = readFileSync(haPetCargoTomlPath, "utf8")
const haPetVersionMatch = haPetCargoToml.match(/^version = "(.*)"$/m)
if (!haPetVersionMatch) {
  console.error("[release:verify] could not read crates/ha-pet/Cargo.toml version")
  process.exit(1)
}

const haVcsCargoToml = readFileSync(haVcsCargoTomlPath, "utf8")
const haVcsVersionMatch = haVcsCargoToml.match(/^version = "(.*)"$/m)
if (!haVcsVersionMatch) {
  console.error("[release:verify] could not read crates/ha-vcs/Cargo.toml version")
  process.exit(1)
}

const cargoLock = readFileSync(cargoLockPath, "utf8")
const cargoLockHopeAgentMatch = cargoLock.match(/name = "hope-agent"\r?\nversion = "(.*)"/)
const cargoLockHaServerMatch = cargoLock.match(/name = "ha-server"\r?\nversion = "(.*)"/)
const cargoLockHaCoreMatch = cargoLock.match(/name = "ha-core"\r?\nversion = "(.*)"/)
const cargoLockHaEvalMatch = cargoLock.match(/name = "ha-eval"\r?\nversion = "(.*)"/)
const cargoLockHaUpdaterMatch = cargoLock.match(/name = "ha-updater"\r?\nversion = "(.*)"/)
const cargoLockHaWeatherMatch = cargoLock.match(/name = "ha-weather"\r?\nversion = "(.*)"/)
const cargoLockHaAcpMatch = cargoLock.match(/name = "ha-acp"\r?\nversion = "(.*)"/)
const cargoLockHaMacMatch = cargoLock.match(/name = "ha-mac"\r?\nversion = "(.*)"/)
const cargoLockHaDesignMatch = cargoLock.match(/name = "ha-design"\r?\nversion = "(.*)"/)
const cargoLockHaBrowserMatch = cargoLock.match(/name = "ha-browser"\r?\nversion = "(.*)"/)
const cargoLockHaVcsMatch = cargoLock.match(/name = "ha-vcs"\r?\nversion = "(.*)"/)
const cargoLockHaMcpMatch = cargoLock.match(/name = "ha-mcp"\r?\nversion = "(.*)"/)
const cargoLockHaPetMatch = cargoLock.match(/name = "ha-pet"\r?\nversion = "(.*)"/)
const cargoLockHaBaseMatch = cargoLock.match(/name = "ha-base"\r?\nversion = "(.*)"/)
const cargoLockHaConfigSchemaMatch = cargoLock.match(
  /name = "ha-config-schema"\r?\nversion = "(.*)"/,
)

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
if (!cargoLockHaEvalMatch) {
  console.error("[release:verify] could not find ha-eval version in Cargo.lock")
  process.exit(1)
}
if (!cargoLockHaBaseMatch) {
  console.error("[release:verify] could not find ha-base version in Cargo.lock")
  process.exit(1)
}
if (!cargoLockHaConfigSchemaMatch) {
  console.error("[release:verify] could not find ha-config-schema version in Cargo.lock")
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
const haBaseVersion = haBaseVersionMatch[1]
const haBaseLockVersion = cargoLockHaBaseMatch[1]
const haConfigSchemaVersion = haConfigSchemaVersionMatch[1]
const haConfigSchemaLockVersion = cargoLockHaConfigSchemaMatch[1]
const browserHostVersion = browserHostVersionMatch[1]
const haEvalVersion = haEvalVersionMatch[1]
const haEvalLockVersion = cargoLockHaEvalMatch[1]
if (!cargoLockHaUpdaterMatch || !cargoLockHaWeatherMatch || !cargoLockHaAcpMatch || !cargoLockHaMacMatch || !cargoLockHaDesignMatch || !cargoLockHaBrowserMatch || !cargoLockHaVcsMatch || !cargoLockHaMcpMatch || !cargoLockHaPetMatch) {
  console.error("[release:verify] Cargo.lock is missing a feature-crate entry (ha-updater / ha-weather / ha-acp / ha-mac / ha-design / ha-browser / ha-vcs / ha-mcp / ha-pet)")
  process.exit(1)
}
const haUpdaterVersion = haUpdaterVersionMatch[1]
const haUpdaterLockVersion = cargoLockHaUpdaterMatch[1]
const haWeatherVersion = haWeatherVersionMatch[1]
const haWeatherLockVersion = cargoLockHaWeatherMatch[1]
const haAcpVersion = haAcpVersionMatch[1]
const haAcpLockVersion = cargoLockHaAcpMatch[1]
const haMacVersion = haMacVersionMatch[1]
const haMacLockVersion = cargoLockHaMacMatch[1]
const haDesignVersion = haDesignVersionMatch[1]
const haDesignLockVersion = cargoLockHaDesignMatch[1]
const haBrowserVersion = haBrowserVersionMatch[1]
const haBrowserLockVersion = cargoLockHaBrowserMatch[1]
const haVcsVersion = haVcsVersionMatch[1]
const haVcsLockVersion = cargoLockHaVcsMatch[1]
const haMcpVersion = haMcpVersionMatch[1]
const haMcpLockVersion = cargoLockHaMcpMatch[1]
const haPetVersion = haPetVersionMatch[1]
const haPetLockVersion = cargoLockHaPetMatch[1]

const mismatches = [
  ["package.json", packageVersion],
  ["src-tauri/tauri.conf.json", tauriVersion],
  ["src-tauri/Cargo.toml", cargoVersion],
  ["Cargo.lock (hope-agent)", cargoLockVersion],
  ["crates/ha-server/Cargo.toml", haServerVersion],
  ["Cargo.lock (ha-server)", haServerLockVersion],
  ["crates/ha-core/Cargo.toml", haCoreVersion],
  ["Cargo.lock (ha-core)", haCoreLockVersion],
  ["crates/ha-base/Cargo.toml", haBaseVersion],
  ["Cargo.lock (ha-base)", haBaseLockVersion],
  ["crates/ha-config-schema/Cargo.toml", haConfigSchemaVersion],
  ["Cargo.lock (ha-config-schema)", haConfigSchemaLockVersion],
  ["crates/ha-browser-host/Cargo.toml", browserHostVersion],
  ["crates/ha-eval/Cargo.toml", haEvalVersion],
  ["Cargo.lock (ha-eval)", haEvalLockVersion],
  ["crates/ha-updater/Cargo.toml", haUpdaterVersion],
  ["Cargo.lock (ha-updater)", haUpdaterLockVersion],
  ["crates/ha-weather/Cargo.toml", haWeatherVersion],
  ["Cargo.lock (ha-weather)", haWeatherLockVersion],
  ["crates/ha-acp/Cargo.toml", haAcpVersion],
  ["Cargo.lock (ha-acp)", haAcpLockVersion],
  ["crates/ha-mac/Cargo.toml", haMacVersion],
  ["Cargo.lock (ha-mac)", haMacLockVersion],
  ["crates/ha-design/Cargo.toml", haDesignVersion],
  ["Cargo.lock (ha-design)", haDesignLockVersion],
  ["crates/ha-browser/Cargo.toml", haBrowserVersion],
  ["Cargo.lock (ha-browser)", haBrowserLockVersion],
  ["crates/ha-vcs/Cargo.toml", haVcsVersion],
  ["Cargo.lock (ha-vcs)", haVcsLockVersion],
  ["crates/ha-mcp/Cargo.toml", haMcpVersion],
  ["Cargo.lock (ha-mcp)", haMcpLockVersion],
  ["crates/ha-pet/Cargo.toml", haPetVersion],
  ["Cargo.lock (ha-pet)", haPetLockVersion],
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
  console.error(`  crates/ha-eval/Cargo.toml: ${haEvalVersion}`)
  console.error(`  Cargo.lock (ha-eval): ${haEvalLockVersion}`)
  console.error(`  crates/ha-updater/Cargo.toml: ${haUpdaterVersion}`)
  console.error(`  Cargo.lock (ha-updater): ${haUpdaterLockVersion}`)
  console.error(`  crates/ha-weather/Cargo.toml: ${haWeatherVersion}`)
  console.error(`  Cargo.lock (ha-weather): ${haWeatherLockVersion}`)
  console.error(`  crates/ha-acp/Cargo.toml: ${haAcpVersion}`)
  console.error(`  Cargo.lock (ha-acp): ${haAcpLockVersion}`)
  console.error(`  crates/ha-mac/Cargo.toml: ${haMacVersion}`)
  console.error(`  Cargo.lock (ha-mac): ${haMacLockVersion}`)
  console.error(`  crates/ha-design/Cargo.toml: ${haDesignVersion}`)
  console.error(`  Cargo.lock (ha-design): ${haDesignLockVersion}`)
  console.error(`  crates/ha-browser/Cargo.toml: ${haBrowserVersion}`)
  console.error(`  Cargo.lock (ha-browser): ${haBrowserLockVersion}`)
  console.error(`  crates/ha-vcs/Cargo.toml: ${haVcsVersion}`)
  console.error(`  Cargo.lock (ha-vcs): ${haVcsLockVersion}`)
  console.error(`  crates/ha-mcp/Cargo.toml: ${haMcpVersion}`)
  console.error(`  Cargo.lock (ha-mcp): ${haMcpLockVersion}`)
  console.error(`  crates/ha-pet/Cargo.toml: ${haPetVersion}`)
  console.error(`  Cargo.lock (ha-pet): ${haPetLockVersion}`)
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
