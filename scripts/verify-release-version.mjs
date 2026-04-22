import { readFileSync } from "node:fs"
import path from "node:path"
import process from "node:process"

const rootDir = process.cwd()
const packageJsonPath = path.join(rootDir, "package.json")
const cargoTomlPath = path.join(rootDir, "src-tauri", "Cargo.toml")
const tauriConfigPath = path.join(rootDir, "src-tauri", "tauri.conf.json")

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

const packageVersion = packageJson.version
const tauriVersion = tauriConfig.version
const cargoVersion = cargoVersionMatch[1]

const mismatches = [
  ["package.json", packageVersion],
  ["src-tauri/tauri.conf.json", tauriVersion],
  ["src-tauri/Cargo.toml", cargoVersion],
].filter(([, value], _, all) => value !== all[0][1])

if (mismatches.length > 0) {
  console.error("[release:verify] version mismatch detected:")
  console.error(`  package.json: ${packageVersion}`)
  console.error(`  src-tauri/tauri.conf.json: ${tauriVersion}`)
  console.error(`  src-tauri/Cargo.toml: ${cargoVersion}`)
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
