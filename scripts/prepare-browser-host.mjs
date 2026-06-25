import { copyFileSync, existsSync, mkdirSync, statSync, chmodSync } from "node:fs"
import { basename, dirname, join, resolve } from "node:path"
import { spawnSync } from "node:child_process"
import { fileURLToPath } from "node:url"

// `--dev` builds a debug host and places it next to the dev binary so
// `pnpm tauri dev` (which does NOT bundle resources) still has a fresh host.
// Without it, `tauri build`'s release flow bundles into src-tauri/resources.
const DEV = process.argv.includes("--dev")

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..")
// Dev always runs the native host alongside the dev binary — never cross-compile.
const targetTriple = DEV
  ? ""
  : process.env.HA_BROWSER_HOST_TARGET ||
    process.env.TAURI_ENV_TARGET_TRIPLE ||
    process.env.CARGO_BUILD_TARGET ||
    inferTargetTriple(process.env.TAURI_PLATFORM, process.env.TAURI_ARCH) ||
    ""
const hostName =
  process.platform === "win32" || targetTriple.includes("windows")
    ? "ha-browser-host.exe"
    : "ha-browser-host"

const profileDir = DEV ? "debug" : "release"
const cargoArgs = ["build", "-p", "ha-browser-host"]
if (!DEV) {
  cargoArgs.push("--release", "--locked")
}
if (targetTriple) {
  cargoArgs.push("--target", targetTriple)
}

const build = spawnSync("cargo", cargoArgs, {
  cwd: repoRoot,
  stdio: "inherit",
  env: process.env,
})

if (build.status !== 0) {
  // In dev, a host build failure must not block the frontend dev server — warn
  // and keep whatever host binary is already on disk.
  if (DEV) {
    console.warn("[prepare-browser-host] dev host build failed; keeping existing binary")
    process.exit(0)
  }
  process.exit(build.status ?? 1)
}

const cargoTargetDir = cargoMetadataTargetDir()
const targetDir = targetTriple
  ? join(cargoTargetDir, targetTriple, profileDir)
  : join(cargoTargetDir, profileDir)
const source = join(targetDir, hostName)
if (!existsSync(source) || !statSync(source).isFile()) {
  console.error(`[prepare-browser-host] missing built host binary: ${source}`)
  process.exit(DEV ? 0 : 1)
}

// Dev: copy next to the dev binary where the running app's current_exe-relative
// host lookup finds it (`<target>/debug/browser-host/`; cargo's direct
// `<target>/debug/<host>` is already fresh too) AND into the Tauri resources
// tree — `pnpm tauri dev` does not bundle resources, but tauri-build still
// validates every declared resource path exists at compile time, so a fresh
// checkout (resources/ is gitignored) would otherwise fail the build script.
// Release: bundle into Tauri resources for packaging.
const resourcesDir = join(repoRoot, "src-tauri", "resources", "browser-host")
const outDirs = DEV ? [join(targetDir, "browser-host"), resourcesDir] : [resourcesDir]
for (const outDir of outDirs) {
  mkdirSync(outDir, { recursive: true })
  const dest = join(outDir, basename(hostName))
  copyFileSync(source, dest)
  if (process.platform !== "win32") {
    chmodSync(dest, 0o755)
  }
  console.log(`[prepare-browser-host] copied ${source} -> ${dest}`)
}

function inferTargetTriple(platform, arch) {
  if (!platform || !arch) return ""
  const normalizedArch = arch === "arm64" ? "aarch64" : arch
  if (platform === "darwin") {
    if (normalizedArch === "x86_64") return "x86_64-apple-darwin"
    if (normalizedArch === "aarch64") return "aarch64-apple-darwin"
  }
  if (platform === "linux") {
    if (normalizedArch === "x86_64") return "x86_64-unknown-linux-gnu"
    if (normalizedArch === "aarch64") return "aarch64-unknown-linux-gnu"
  }
  if (platform === "windows") {
    if (normalizedArch === "x86_64") return "x86_64-pc-windows-msvc"
    if (normalizedArch === "aarch64") return "aarch64-pc-windows-msvc"
  }
  return ""
}

function cargoMetadataTargetDir() {
  const metadata = spawnSync("cargo", ["metadata", "--format-version=1", "--no-deps"], {
    cwd: repoRoot,
    encoding: "utf8",
    env: process.env,
  })
  if (metadata.status !== 0) {
    process.stderr.write(metadata.stderr || "")
    console.error("[prepare-browser-host] cargo metadata failed")
    process.exit(metadata.status ?? 1)
  }
  try {
    const parsed = JSON.parse(metadata.stdout)
    if (typeof parsed.target_directory === "string" && parsed.target_directory) {
      return parsed.target_directory
    }
  } catch (error) {
    console.error(`[prepare-browser-host] parsing cargo metadata failed: ${error}`)
    process.exit(1)
  }
  console.error("[prepare-browser-host] cargo metadata did not include target_directory")
  process.exit(1)
}
