import { readFileSync, writeFileSync } from "node:fs"
import { execSync } from "node:child_process"
import path from "node:path"
import process from "node:process"

const rootDir = process.cwd()
const packageJsonPath = path.join(rootDir, "package.json")
const tauriCargoTomlPath = path.join(rootDir, "src-tauri", "Cargo.toml")
const tauriConfigPath = path.join(rootDir, "src-tauri", "tauri.conf.json")
// ha-server ships its own `hope-agent` binary in the Docker image — that
// binary reads `env!("CARGO_PKG_VERSION")` from this crate's manifest, so
// the version must move in lockstep with the desktop binary or
// `--version` / `app_update` will report the wrong number in containers.
const haServerCargoTomlPath = path.join(rootDir, "crates", "ha-server", "Cargo.toml")

const packageJson = JSON.parse(readFileSync(packageJsonPath, "utf8"))
const version = packageJson.version

if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/.test(version)) {
  console.error(`[sync-version] package.json version is not valid semver: ${version}`)
  process.exit(1)
}

const tauriConfig = JSON.parse(readFileSync(tauriConfigPath, "utf8"))
tauriConfig.version = version
writeFileSync(tauriConfigPath, `${JSON.stringify(tauriConfig, null, 2)}\n`)

function bumpCargoTomlVersion(filePath, label) {
  const current = readFileSync(filePath, "utf8")
  const next = current.replace(/^version = ".*"$/m, `version = "${version}"`)
  if (next === current) {
    console.error(`[sync-version] failed to update ${label} version`)
    process.exit(1)
  }
  writeFileSync(filePath, next)
}

bumpCargoTomlVersion(tauriCargoTomlPath, "src-tauri/Cargo.toml")
bumpCargoTomlVersion(haServerCargoTomlPath, "crates/ha-server/Cargo.toml")

// hope-agent / ha-server are workspace packages; cargo update only bumps
// the Cargo.lock entries to match the new manifest version. `--offline`
// keeps the script working with no network. Skipping any of these would
// make CI's `cargo clippy --locked` reject the version-bump commit.
try {
  execSync("cargo update -p hope-agent -p ha-server --offline --quiet", {
    cwd: rootDir,
    stdio: "inherit",
  })
} catch {
  console.error(
    "[sync-version] failed to sync Cargo.lock; ensure Rust toolchain is installed, or run `cargo update -p hope-agent -p ha-server` manually",
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
      "git add package.json src-tauri/Cargo.toml src-tauri/tauri.conf.json crates/ha-server/Cargo.toml Cargo.lock",
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
console.log(
  "[sync-version] updated: src-tauri/Cargo.toml, src-tauri/tauri.conf.json, crates/ha-server/Cargo.toml, Cargo.lock",
)
