import { readFileSync, writeFileSync } from "node:fs"
import { execSync } from "node:child_process"
import path from "node:path"
import process from "node:process"

const rootDir = process.cwd()
const packageJsonPath = path.join(rootDir, "package.json")
const cargoTomlPath = path.join(rootDir, "src-tauri", "Cargo.toml")
const tauriConfigPath = path.join(rootDir, "src-tauri", "tauri.conf.json")

const packageJson = JSON.parse(readFileSync(packageJsonPath, "utf8"))
const version = packageJson.version

if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/.test(version)) {
  console.error(`[sync-version] package.json version is not valid semver: ${version}`)
  process.exit(1)
}

const tauriConfig = JSON.parse(readFileSync(tauriConfigPath, "utf8"))
tauriConfig.version = version
writeFileSync(tauriConfigPath, `${JSON.stringify(tauriConfig, null, 2)}\n`)

const cargoToml = readFileSync(cargoTomlPath, "utf8")
const nextCargoToml = cargoToml.replace(/^version = ".*"$/m, `version = "${version}"`)

if (nextCargoToml === cargoToml) {
  console.error("[sync-version] failed to update src-tauri/Cargo.toml version")
  process.exit(1)
}

writeFileSync(cargoTomlPath, nextCargoToml)

// hope-agent 是 workspace package，cargo update 只 bump Cargo.lock 里的 workspace 版本，
// --offline 避免查 registry。漏掉这步会让 CI 的 `cargo clippy --locked` 卡住。
try {
  execSync("cargo update -p hope-agent --offline --quiet", {
    cwd: rootDir,
    stdio: "inherit",
  })
} catch {
  console.error(
    "[sync-version] failed to sync Cargo.lock; ensure Rust toolchain is installed, or run `cargo update -p hope-agent` manually",
  )
  process.exit(1)
}

if (process.env.npm_lifecycle_event === "version") {
  try {
    execSync("git rev-parse --is-inside-work-tree", {
      cwd: rootDir,
      stdio: "ignore",
    })
    execSync(
      "git add package.json src-tauri/Cargo.toml src-tauri/tauri.conf.json Cargo.lock",
      {
        cwd: rootDir,
        stdio: "ignore",
      },
    )
  } catch {
    // Non-git environments can still use the sync script without staging.
  }
}

console.log(`[sync-version] synced desktop version to ${version}`)
console.log("[sync-version] updated: src-tauri/Cargo.toml, src-tauri/tauri.conf.json, Cargo.lock")
